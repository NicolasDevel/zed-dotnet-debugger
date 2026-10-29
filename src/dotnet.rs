use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use zed_extension_api::{
    self as zed, BuildTaskDefinition, BuildTaskDefinitionTemplatePayload, BuildTaskTemplate,
    DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario, DebugTaskDefinition,
    DownloadedFileType, EnvVars, LaunchRequest, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, TaskTemplate, Worktree,
};

const ADAPTER_NAME: &str = "netcoredbg";
const LOCATOR_NAME: &str = "dotnet";
const NETCOREDBG_REPO: &str = "Samsung/netcoredbg";
/// Optional helper script installed by `install.sh`; its tasks look like `sh <path>/dotnet-zed.sh <action> <root>`.
const HELPER_SCRIPT: &str = "dotnet-zed.sh";
/// Written by the debug build (`-getResultOutputFile`) into the extension's work directory.
const BUILD_RESULT_FILE: &str = "last-build.json";

struct DotnetExtension {
    cached_netcoredbg: Option<String>,
}

/// What a `dotnet run|watch|build` task means for a debug session.
#[derive(Default)]
struct DotnetInvocation {
    project: Option<String>,
    framework: Option<String>,
    /// `Some(None)` is `--no-launch-profile`.
    launch_profile: Option<Option<String>>,
    program_args: Vec<String>,
}

impl DotnetInvocation {
    /// Parses `dotnet run|watch|build [options] [-- args]`, given as command + args.
    fn parse(task: &TaskTemplate) -> Option<Self> {
        let mut tokens = task
            .command
            .split_whitespace()
            .map(String::from)
            .chain(task.args.iter().cloned())
            .peekable();
        let program = tokens.next()?;
        if program != "dotnet" && !program.ends_with("/dotnet") && !program.ends_with("\\dotnet.exe")
        {
            return None;
        }
        let verb = match tokens.next()?.as_str() {
            "watch" => {
                // `dotnet watch [run|build]`
                if tokens.peek().is_some_and(|t| t == "run" || t == "build") {
                    tokens.next();
                }
                "run"
            }
            "run" => "run",
            "build" => "build",
            _ => return None,
        };

        let mut invocation = Self::default();
        while let Some(token) = tokens.next() {
            let (flag, inline_value) = match token.split_once('=') {
                Some((flag, value)) if flag.starts_with('-') => {
                    (flag.to_string(), Some(value.to_string()))
                }
                _ => (token.clone(), None),
            };
            let mut value = || inline_value.clone().or_else(|| tokens.next());
            match flag.as_str() {
                "--" => invocation.program_args.extend(tokens.by_ref()),
                "-p" | "--project" => invocation.project = value(),
                "-f" | "--framework" => invocation.framework = value(),
                "-lp" | "--launch-profile" => invocation.launch_profile = Some(value()),
                "--no-launch-profile" => invocation.launch_profile = Some(None),
                "-c" | "--configuration" | "-r" | "--runtime" | "-a" | "--arch" | "--os"
                | "-v" | "--verbosity" | "-o" | "--output" | "-e" | "--environment" => {
                    value();
                }
                _ if flag.starts_with('-') => {}
                // `dotnet build <PROJECT>`; for `dotnet run`, stray positionals go to the app.
                _ if verb == "build" && invocation.project.is_none() => {
                    invocation.project = Some(token)
                }
                _ if verb == "run" => invocation.program_args.push(token),
                _ => {}
            }
        }
        Some(invocation)
    }

    /// Extra debug configuration, merged on top of what the locator resolves.
    fn config(&self) -> Value {
        let mut config = json!({});
        if !self.program_args.is_empty() {
            config["args"] = json!(self.program_args);
        }
        match &self.launch_profile {
            Some(Some(name)) => config["launchProfile"] = name.clone().into(),
            Some(None) => config["launchProfile"] = false.into(),
            None => {}
        }
        config
    }
}

fn build_result_path() -> Result<PathBuf, String> {
    Ok(std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(BUILD_RESULT_FILE))
}

/// MSBuild flags that make `dotnet build` report where the program ended up.
fn build_result_args() -> Result<Vec<String>, String> {
    Ok(vec![
        "-c".into(),
        "Debug".into(),
        // Without an explicit target, -getProperty only evaluates the project.
        "-t:Build".into(),
        "-getProperty:TargetPath".into(),
        "-getProperty:MSBuildProjectDirectory".into(),
        format!("-getResultOutputFile:{}", build_result_path()?.display()),
    ])
}

fn dotnet_build_task(
    project: Option<String>,
    framework: Option<String>,
    cwd: Option<String>,
    env: EnvVars,
) -> Result<BuildTaskTemplate, String> {
    let mut args = vec!["build".to_string()];
    args.extend(project);
    if let Some(framework) = framework {
        args.extend(["-f".to_string(), framework]);
    }
    args.extend(build_result_args()?);
    Ok(BuildTaskTemplate {
        label: "dotnet: build (debug)".into(),
        command: "dotnet".into(),
        args,
        env,
        cwd,
    })
}

fn debug_scenario(label: String, build: BuildTaskTemplate, config: Value) -> DebugScenario {
    DebugScenario {
        label,
        adapter: ADAPTER_NAME.into(),
        build: Some(BuildTaskDefinition::Template(
            BuildTaskDefinitionTemplatePayload {
                locator_name: Some(LOCATOR_NAME.into()),
                template: build,
            },
        )),
        config: config.to_string(),
        tcp_connection: None,
    }
}

/// Returns the helper script path and action if `task` is one of `install.sh`'s tasks.
fn helper_invocation(task: &TaskTemplate) -> Option<(&str, &str)> {
    if task.command != "sh" {
        return None;
    }
    let script = task.args.first()?;
    let action = task.args.get(1)?;
    script
        .ends_with(HELPER_SCRIPT)
        .then_some((script.as_str(), action.as_str()))
}

fn request_kind(config: &Value) -> Result<StartDebuggingRequestArgumentsRequest, String> {
    match config.get("request").and_then(Value::as_str) {
        Some("launch") if config.get("program").is_some() => {
            Ok(StartDebuggingRequestArgumentsRequest::Launch)
        }
        Some("launch") => Err("`program` is required for a netcoredbg launch".into()),
        Some("attach") => Ok(StartDebuggingRequestArgumentsRequest::Attach),
        Some(other) => Err(format!("unknown request `{other}`, expected launch or attach")),
        None => Err("missing `request` (launch or attach)".into()),
    }
}

/// Parses launchSettings.json as Visual Studio writes it: UTF-8 BOM and `//` comment lines allowed.
fn parse_launch_settings(text: &str) -> Option<Value> {
    let text = text.trim_start_matches('\u{feff}');
    let without_comments: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&without_comments).ok()
}

/// Applies the `Properties/launchSettings.json` profile of the program's `cwd`, like `dotnet run`.
///
/// `launchProfile` picks a profile by name (default: the first `Project` profile) or turns this
/// off with `false`. Values already present in the configuration win over the profile.
fn apply_launch_profile(configuration: &mut Value, worktree: &Worktree) {
    let selection = configuration
        .as_object_mut()
        .and_then(|config| config.remove("launchProfile"));
    if selection == Some(Value::Bool(false))
        || configuration.get("request").and_then(Value::as_str) != Some("launch")
    {
        return;
    }
    let Some(cwd) = configuration.get("cwd").and_then(Value::as_str) else {
        return;
    };
    let root = worktree.root_path();
    let Some(relative) = cwd.strip_prefix(root.as_str()) else {
        return;
    };
    let relative = relative.trim_start_matches(['/', '\\']);
    let path = if relative.is_empty() {
        "Properties/launchSettings.json".to_string()
    } else {
        format!("{relative}/Properties/launchSettings.json")
    };
    let Some(settings) = worktree
        .read_text_file(&path)
        .ok()
        .and_then(|text| parse_launch_settings(&text))
    else {
        return;
    };
    let Some(profiles) = settings.get("profiles").and_then(Value::as_object) else {
        return;
    };
    let profile = match selection.as_ref().and_then(Value::as_str) {
        Some(name) => profiles.get(name),
        None => profiles
            .values()
            .find(|p| p.get("commandName").and_then(Value::as_str) == Some("Project")),
    };
    let Some(profile) = profile else {
        return;
    };

    let mut env: Map<String, Value> = profile
        .get("environmentVariables")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(urls) = profile.get("applicationUrl").and_then(Value::as_str) {
        env.entry("ASPNETCORE_URLS").or_insert_with(|| urls.into());
    }
    if let Some(existing) = configuration.get("env").and_then(Value::as_object) {
        env.extend(existing.clone());
    }
    if !env.is_empty() {
        configuration["env"] = env.into();
    }

    let has_args = configuration
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| !args.is_empty());
    if !has_args && let Some(args) = profile.get("commandLineArgs").and_then(Value::as_str) {
        configuration["args"] = json!(args.split_whitespace().collect::<Vec<_>>());
    }
}

impl DotnetExtension {
    fn netcoredbg_path(
        &mut self,
        user_provided_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<String, String> {
        if let Some(path) = user_provided_path {
            return Ok(path);
        }
        if let Some(path) = worktree.which(ADAPTER_NAME) {
            return Ok(path);
        }
        if let Some(path) = &self.cached_netcoredbg
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        let (os, arch) = zed::current_platform();
        let (asset_name, file_type) = match (os, arch) {
            (zed::Os::Mac, zed::Architecture::Aarch64) => {
                ("netcoredbg-osx-arm64.zip", DownloadedFileType::Zip)
            }
            (zed::Os::Mac, _) => ("netcoredbg-osx-amd64.tar.gz", DownloadedFileType::GzipTar),
            (zed::Os::Linux, zed::Architecture::Aarch64) => {
                ("netcoredbg-linux-arm64.tar.gz", DownloadedFileType::GzipTar)
            }
            (zed::Os::Linux, _) => ("netcoredbg-linux-amd64.tar.gz", DownloadedFileType::GzipTar),
            (zed::Os::Windows, _) => ("netcoredbg-win64.zip", DownloadedFileType::Zip),
        };
        let binary_name = match os {
            zed::Os::Windows => "netcoredbg.exe",
            _ => "netcoredbg",
        };

        let release = zed::latest_github_release(
            NETCOREDBG_REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "netcoredbg {} has no `{asset_name}` asset; install netcoredbg manually and put it on PATH",
                    release.version
                )
            })?;

        let version_dir = format!("netcoredbg-{}", release.version);
        let binary_path = format!("{version_dir}/netcoredbg/{binary_name}");

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("failed to download netcoredbg: {e}"))?;
            zed::make_file_executable(&binary_path)?;

            if let Ok(entries) = fs::read_dir(".") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with("netcoredbg-") && name != version_dir {
                        fs::remove_dir_all(entry.path()).ok();
                    }
                }
            }
        }

        // Zed launches the adapter from the project root, so the path must be absolute.
        let binary_path = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(&binary_path)
            .to_string_lossy()
            .into_owned();
        self.cached_netcoredbg = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for DotnetExtension {
    fn new() -> Self {
        Self {
            cached_netcoredbg: None,
        }
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<DebugAdapterBinary, String> {
        if adapter_name != ADAPTER_NAME {
            return Err(format!("unknown debug adapter `{adapter_name}`"));
        }
        let mut configuration: Value = serde_json::from_str(&config.config)
            .map_err(|e| format!("invalid netcoredbg configuration: {e}"))?;
        let request = request_kind(&configuration)?;
        apply_launch_profile(&mut configuration, worktree);
        let command = self.netcoredbg_path(user_provided_debug_adapter_path, worktree)?;

        Ok(DebugAdapterBinary {
            command: Some(command),
            arguments: vec!["--interpreter=vscode".into()],
            envs: Vec::new(),
            cwd: Some(worktree.root_path()),
            connection: None,
            request_args: StartDebuggingRequestArguments {
                configuration: configuration.to_string(),
                request,
            },
        })
    }

    fn dap_request_kind(
        &mut self,
        _adapter_name: String,
        config: Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        request_kind(&config)
    }

    fn dap_config_to_scenario(&mut self, config: DebugConfig) -> Result<DebugScenario, String> {
        let configuration = match config.request {
            // A project rather than a compiled program: build it, then debug its output.
            DebugRequest::Launch(launch)
                if launch.program.ends_with(".csproj") || launch.program.ends_with(".fsproj") =>
            {
                let build =
                    dotnet_build_task(Some(launch.program), None, launch.cwd, launch.envs)?;
                let mut extra = json!({ "stopAtEntry": config.stop_on_entry.unwrap_or(false) });
                if !launch.args.is_empty() {
                    extra["args"] = json!(launch.args);
                }
                return Ok(debug_scenario(config.label, build, extra));
            }
            DebugRequest::Launch(launch) => {
                let mut value = json!({
                    "request": "launch",
                    "program": launch.program,
                    "args": launch.args,
                    "stopAtEntry": config.stop_on_entry.unwrap_or(false),
                    "justMyCode": true,
                });
                if let Some(cwd) = launch.cwd {
                    value["cwd"] = cwd.into();
                }
                if !launch.envs.is_empty() {
                    let env: Map<String, Value> = launch
                        .envs
                        .into_iter()
                        .map(|(key, value)| (key, value.into()))
                        .collect();
                    value["env"] = env.into();
                }
                value
            }
            DebugRequest::Attach(attach) => {
                let mut value = json!({ "request": "attach" });
                if let Some(pid) = attach.process_id {
                    value["processId"] = pid.into();
                }
                value
            }
        };

        Ok(DebugScenario {
            label: config.label,
            adapter: config.adapter,
            build: None,
            config: configuration.to_string(),
            tcp_connection: None,
        })
    }

    fn dap_locator_create_scenario(
        &mut self,
        locator_name: String,
        build_task: TaskTemplate,
        resolved_label: String,
        _debug_adapter_name: String,
    ) -> Option<DebugScenario> {
        if locator_name != LOCATOR_NAME {
            return None;
        }

        if let Some((script, action)) = helper_invocation(&build_task) {
            // Only `run`: watch/build/stop would debug the same program, or nothing.
            if action != "run" {
                return None;
            }
            let mut args = vec![script.to_string(), "build".to_string()];
            args.extend(build_task.args.get(2).cloned());
            args.extend(build_result_args().ok()?);
            let build = BuildTaskTemplate {
                label: "dotnet: build (debug)".into(),
                command: build_task.command.clone(),
                args,
                env: build_task.env.clone(),
                cwd: build_task.cwd.clone(),
            };
            return Some(debug_scenario("dotnet: debug".into(), build, json!({})));
        }

        let invocation = DotnetInvocation::parse(&build_task)?;
        let build = dotnet_build_task(
            invocation.project.clone(),
            invocation.framework.clone(),
            build_task.cwd.clone(),
            build_task.env.clone(),
        )
        .ok()?;
        Some(debug_scenario(
            format!("Debug: {resolved_label}"),
            build,
            invocation.config(),
        ))
    }

    fn run_dap_locator(
        &mut self,
        _locator_name: String,
        _build_task: TaskTemplate,
    ) -> Result<DebugRequest, String> {
        let path = build_result_path()?;
        let text = fs::read_to_string(&path).map_err(|_| {
            "the build did not report its output; the .NET SDK 8 or newer is required".to_string()
        })?;
        fs::remove_file(&path).ok();

        let result: Value = serde_json::from_str(&text)
            .map_err(|e| format!("unexpected MSBuild result file: {e}"))?;
        let properties = &result["Properties"];
        let program = properties["TargetPath"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or(
                "MSBuild did not report a TargetPath: build a single project (not a solution), \
                 and pass -f <framework> if it targets several",
            )?
            .to_string();
        let cwd = properties["MSBuildProjectDirectory"]
            .as_str()
            .map(String::from);

        Ok(DebugRequest::Launch(LaunchRequest {
            program,
            cwd,
            args: Vec::new(),
            envs: Vec::new(),
        }))
    }
}

zed::register_extension!(DotnetExtension);

#[cfg(test)]
mod tests {
    use super::*;

    fn task(command: &str, args: &[&str]) -> TaskTemplate {
        TaskTemplate {
            label: "test".into(),
            command: command.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: Vec::new(),
            cwd: None,
        }
    }

    #[test]
    fn parses_run_with_project_profile_and_program_args() {
        let invocation = DotnetInvocation::parse(&task(
            "dotnet",
            &["run", "--project", "Api", "-c", "Release", "-lp", "https", "--", "--seed", "1"],
        ))
        .unwrap();
        assert_eq!(invocation.project.as_deref(), Some("Api"));
        assert_eq!(invocation.program_args, ["--seed", "1"]);
        assert_eq!(
            invocation.config(),
            json!({ "args": ["--seed", "1"], "launchProfile": "https" })
        );
    }

    #[test]
    fn parses_command_with_inline_verb_like_the_csharp_extension() {
        let invocation =
            DotnetInvocation::parse(&task("dotnet build", &["/repo/Api/Api.csproj"])).unwrap();
        assert_eq!(invocation.project.as_deref(), Some("/repo/Api/Api.csproj"));
        assert!(invocation.program_args.is_empty());
    }

    #[test]
    fn parses_watch_and_inline_option_values() {
        let invocation = DotnetInvocation::parse(&task(
            "dotnet",
            &["watch", "run", "--project=Api", "--framework=net8.0", "--no-launch-profile"],
        ))
        .unwrap();
        assert_eq!(invocation.project.as_deref(), Some("Api"));
        assert_eq!(invocation.framework.as_deref(), Some("net8.0"));
        assert_eq!(invocation.config(), json!({ "launchProfile": false }));
    }

    #[test]
    fn ignores_other_commands() {
        assert!(DotnetInvocation::parse(&task("dotnet", &["test"])).is_none());
        assert!(DotnetInvocation::parse(&task("cargo", &["run"])).is_none());
        assert!(DotnetInvocation::parse(&task("dotnet restore", &["x.csproj"])).is_none());
    }

    #[test]
    fn helper_tasks_are_recognized() {
        let helper = task("sh", &["/home/u/.config/zed/dotnet/dotnet-zed.sh", "run", "/repo"]);
        assert_eq!(
            helper_invocation(&helper),
            Some(("/home/u/.config/zed/dotnet/dotnet-zed.sh", "run"))
        );
        assert!(DotnetInvocation::parse(&helper).is_none());
    }

    #[test]
    fn launch_settings_with_bom_and_comments() {
        let text = "\u{feff}{\n  // comment\n  \"profiles\": { \"http\": { \"commandName\": \"Project\" } }\n}";
        let settings = parse_launch_settings(text).unwrap();
        assert_eq!(settings["profiles"]["http"]["commandName"], "Project");
    }
}
