# .NET Debugger for Zed

Run, stop, restart and debug .NET (C# / F#) projects in [Zed](https://zed.dev), Visual Studio style.

- **Debug adapter `netcoredbg`** — Samsung's open-source .NET debugger. Downloaded automatically on first use
  (macOS arm64/x64, Linux arm64/x64, Windows x64), or the one on your `PATH` is used.
- **Debug locator `dotnet`** — turns any `dotnet run`, `dotnet watch` or `dotnet build` task into a debug
  session: it builds the project in Debug, MSBuild reports the output `.dll`, and netcoredbg launches it.
- **launchSettings.json** — environment variables, `applicationUrl` and `commandLineArgs` come from the first
  `Project` profile in `Properties/launchSettings.json` (or the one picked with `--launch-profile`), like
  `dotnet run` and Visual Studio.
- **Optional run / watch / stop / build tasks and keybindings**, installed by `install.sh`.

Requires the .NET SDK 8 or newer (projects may target older frameworks). The extension runs no commands of its own:
it only downloads netcoredbg from GitHub.

## Install

1. Install **.NET Debugger** from `zed: extensions` (or, from source, `zed: install dev extension` and pick
   this folder; that needs Rust via `rustup`).
2. Make netcoredbg the debugger for C# in `settings.json`:

   ```jsonc
   "languages": {
     "CSharp": { "debuggers": ["netcoredbg"] }
   }
   ```

That is enough to debug: press `F5`, open the **Launch** tab and enter the path to a `.csproj` — it is built and
debugged. Every `dotnet run` / `watch` / `build` task also shows up in the **Debug** tab as `Debug: <task>`, for
example from `.zed/tasks.json`:

```jsonc
[{ "label": "dotnet run", "command": "dotnet", "args": ["run", "--project", "src/Api"] }]
```

### Optional: tasks and keybindings

```sh
./install.sh
```

It copies `scripts/dotnet-zed.sh` to `~/.config/zed/dotnet/` and merges `config/tasks.json` and
`config/keymap.json` into `~/.config/zed/`. These tasks find the project from the open file, and `run` restarts
the app if it is already running. Re-running the script replaces its own entries and leaves the rest alone.

## Usage

Keep a `.cs` file focused: Zed only offers the C# debugger when the active file is C#, and the `install.sh` tasks
resolve the project from it.

**Debugging** — function keys, as in Visual Studio (`fn` + key on Mac keyboards):

| Action | Key |
|---|---|
| Pick what to start (Debug tab → **dotnet: debug**, Task tab → tasks) | `F5` |
| Continue (stopped at a breakpoint) | `F5` |
| Stop debugging | `shift-F5` |
| Restart debugging | `shift-cmd-F5` |
| Toggle breakpoint | `F9` |
| Step over / into / out | `F10` / `F11` / `shift-F11` |

**Running without the debugger** (with `install.sh`) — `cmd-alt-shift` + letter (no `fn`, no clashes with Zed defaults):

| Action | Key |
|---|---|
| Run (restarts it if it is already running) | `cmd-alt-shift-R` |
| Watch (hot reload) | `cmd-alt-shift-W` |
| Stop | `cmd-alt-shift-X` |
| Build | `cmd-alt-shift-B` |

All tasks are also available from `task: spawn` (`cmd-shift-r`).

> Debugging uses the same port as `dotnet: run`. Stop the running app (`cmd-alt-shift-X`) before pressing `F5`.

### Which project is used?

For a plain `dotnet` task, the one it names (`--project`, or the path given to `dotnet build`), else the one in
its working directory. For the `install.sh` tasks, the nearest `*.csproj` / `*.fsproj` upwards from the open
file; if there is none, the workspace is searched and an executable project (Web / Worker / `OutputType` Exe) is
preferred. Set the `DOTNET_PROJECT` environment variable on a task to force one.

### Manual `debug.json`

```jsonc
// .zed/debug.json
[
  {
    "label": "Debug Api",
    "adapter": "netcoredbg",
    "request": "launch",
    "program": "$ZED_WORKTREE_ROOT/Api/bin/Debug/net10.0/Api.dll",
    "cwd": "$ZED_WORKTREE_ROOT/Api",
    "env": { "ASPNETCORE_ENVIRONMENT": "Development" },
    "build": { "command": "dotnet", "args": ["build", "Api"] }
  },
  {
    "label": "Attach to .NET process",
    "adapter": "netcoredbg",
    "request": "attach",
    "processId": "$ZED_PICK_PID"
  }
]
```

Supported options: `program`, `args`, `cwd`, `env`, `launchProfile`, `stopAtEntry`, `justMyCode`,
`enableStepFiltering`, `processId` (see `debug_adapter_schemas/netcoredbg.json`).

## Development

```sh
cargo test                                   # parser tests (host)
cargo build --release --target wasm32-wasip2
```

After changing the Rust code, use **Rebuild** on the extension in `zed: extensions`. Logs: `zed: open log`.

## License

MIT. netcoredbg is © Samsung Electronics, MIT licensed, and is downloaded from its
[GitHub releases](https://github.com/Samsung/netcoredbg/releases).
