use std::fs;

use serde_json::{Map, Value, json};
use zed_extension_api::{
    self as zed, BuildTaskDefinition, BuildTaskDefinitionTemplatePayload, BuildTaskTemplate,
    DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario, DebugTaskDefinition,
    DownloadedFileType, LaunchRequest, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, TaskTemplate, Worktree,
};

const ADAPTER_NAME: &str = "netcoredbg";
const LOCATOR_NAME: &str = "dotnet";
const NETCOREDBG_REPO: &str = "Samsung/netcoredbg";
/// Helper script installed by `install.sh`; tasks invoke it as `sh <path>/dotnet-zed.sh <action> ...`.
const HELPER_SCRIPT: &str = "dotnet-zed.sh";
/// Only `run` gets a debug scenario; `watch`/`restart` would debug the exact same program.
const DEBUGGABLE_ACTIONS: &[&str] = &["run"];

struct DotnetExtension {
    cached_netcoredbg: Option<String>,
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

/// Returns the helper script path and action if `task` is one of our `dotnet-zed.sh` tasks.
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

/// Builds the env/args VS would use from the first `Project` profile in launchSettings.json.
fn launch_profile(launch_settings: &Value) -> (Vec<(String, String)>, Vec<String>) {
    let mut envs = Vec::new();
    let mut args = Vec::new();
    let Some(profile) = launch_settings
        .get("profiles")
        .and_then(Value::as_object)
        .and_then(|profiles| {
            profiles
                .values()
                .find(|p| p.get("commandName").and_then(Value::as_str) == Some("Project"))
        })
    else {
        return (envs, args);
    };

    if let Some(vars) = profile.get("environmentVariables").and_then(Value::as_object) {
        for (key, value) in vars {
            if let Some(value) = value.as_str() {
                envs.push((key.clone(), value.to_string()));
            }
        }
    }
    if let Some(urls) = profile.get("applicationUrl").and_then(Value::as_str)
        && !envs.iter().any(|(key, _)| key == "ASPNETCORE_URLS")
    {
        envs.push(("ASPNETCORE_URLS".into(), urls.to_string()));
    }
    if let Some(cmd_args) = profile.get("commandLineArgs").and_then(Value::as_str) {
        args.extend(cmd_args.split_whitespace().map(String::from));
    }
    (envs, args)
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
        let configuration: Value = serde_json::from_str(&config.config)
            .map_err(|e| format!("invalid netcoredbg configuration: {e}"))?;
        let request = request_kind(&configuration)?;
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
        _resolved_label: String,
        _debug_adapter_name: String,
    ) -> Option<DebugScenario> {
        if locator_name != LOCATOR_NAME {
            return None;
        }
        let (script, action) = helper_invocation(&build_task)?;
        if !DEBUGGABLE_ACTIONS.contains(&action) {
            return None;
        }

        // Same invocation, but only build; the locator resolves the output afterwards.
        let mut args = vec![script.to_string(), "build".to_string()];
        args.extend(build_task.args.iter().skip(2).cloned());
        let build = BuildTaskTemplate {
            label: "dotnet: build (debug)".into(),
            command: build_task.command.clone(),
            args,
            env: build_task.env.clone(),
            cwd: build_task.cwd.clone(),
        };

        Some(DebugScenario {
            label: "dotnet: debug".into(),
            adapter: ADAPTER_NAME.into(),
            build: Some(BuildTaskDefinition::Template(
                BuildTaskDefinitionTemplatePayload {
                    locator_name: Some(LOCATOR_NAME.into()),
                    template: build,
                },
            )),
            config: "{}".into(),
            tcp_connection: None,
        })
    }

    fn run_dap_locator(
        &mut self,
        _locator_name: String,
        build_task: TaskTemplate,
    ) -> Result<DebugRequest, String> {
        let (script, _) = helper_invocation(&build_task)
            .ok_or("not a dotnet-zed task; cannot locate the program to debug")?;
        let root = build_task.args.get(2).cloned().unwrap_or_default();
        // Args after `<script> build <root>` are forwarded to the program, like `dotnet run`.
        let task_args: Vec<String> = build_task.args.iter().skip(3).cloned().collect();
        let cwd = build_task.cwd.clone().unwrap_or_else(|| root.clone());

        let output = zed::process::Command::new("sh")
            .args([
                "-c",
                "cd \"$0\" && exec sh \"$1\" info \"$2\"",
                cwd.as_str(),
                script,
                root.as_str(),
            ])
            .envs(build_task.env.clone())
            .output()?;
        if output.status != Some(0) {
            return Err(format!(
                "failed to resolve the .NET project: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let (msbuild, launch_settings) = stdout.split_once('\n').unwrap_or((&stdout, ""));
        let msbuild: Value = serde_json::from_str(msbuild)
            .map_err(|e| format!("unexpected MSBuild output from {HELPER_SCRIPT} info: {e}"))?;
        let properties = &msbuild["Properties"];
        let program = properties["TargetPath"]
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or("MSBuild did not report a TargetPath for the project")?
            .to_string();
        let project_dir = properties["MSBuildProjectDirectory"]
            .as_str()
            .map(String::from);

        let (mut envs, mut args) = parse_launch_settings(launch_settings)
            .map(|settings| launch_profile(&settings))
            .unwrap_or_default();
        if !envs.iter().any(|(key, _)| key == "DOTNET_ENVIRONMENT") {
            envs.push(("DOTNET_ENVIRONMENT".into(), "Development".into()));
        }
        if !envs.iter().any(|(key, _)| key == "ASPNETCORE_ENVIRONMENT") {
            envs.push(("ASPNETCORE_ENVIRONMENT".into(), "Development".into()));
        }
        if !task_args.is_empty() {
            args = task_args;
        }

        Ok(DebugRequest::Launch(LaunchRequest {
            program,
            cwd: project_dir,
            args,
            envs,
        }))
    }
}

zed::register_extension!(DotnetExtension);
