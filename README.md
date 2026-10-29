# .NET Debugger for Zed

Run, stop, restart and debug .NET (C# / F#) projects in [Zed](https://zed.dev), Visual Studio style.

- **Debug adapter `netcoredbg`** — Samsung's open-source .NET debugger. Downloaded automatically on first use
  (macOS arm64/x64, Linux arm64/x64, Windows x64), or the one on your `PATH` is used.
- **Debug locator `dotnet`** — turns the `dotnet: run` task into a debug session: builds in Debug, asks MSBuild
  for the output `.dll`, and launches it under netcoredbg. Environment variables, `applicationUrl` and
  `commandLineArgs` are taken from the first `Project` profile in `Properties/launchSettings.json`,
  like Visual Studio does.
- **Run / watch / stop / build tasks** and **keybindings**, installed by `install.sh`.

## Install

1. Install the extension. Until it is in the registry: `zed: install dev extension` and pick this folder
   (needs Rust via `rustup`, with `rustc` on Zed's `PATH`).
2. Install the tasks, helper script and keybindings:

   ```sh
   ./install.sh
   ```

   It copies `scripts/dotnet-zed.sh` to `~/.config/zed/dotnet/` and merges the entries from `config/` into
   `~/.config/zed/tasks.json` and `~/.config/zed/keymap.json`. Re-running it replaces its own entries and
   leaves everything else alone.

3. Make netcoredbg the debugger for C# in `settings.json`:

   ```jsonc
   "languages": {
     "CSharp": { "debuggers": ["netcoredbg"] }
   }
   ```

## Usage

Keep a `.cs` file focused: the project is resolved from it, and Zed only offers the C# debugger when the active
file is C#.

**Debugging** — function keys, as in Visual Studio (`fn` + key on Mac keyboards):

| Action | Key |
|---|---|
| Pick what to start (Debug tab → **dotnet: debug**, Task tab → tasks) | `F5` |
| Continue (stopped at a breakpoint) | `F5` |
| Stop debugging | `shift-F5` |
| Restart debugging | `shift-cmd-F5` |
| Toggle breakpoint | `F9` |
| Step over / into / out | `F10` / `F11` / `shift-F11` |

**Running without the debugger** — `cmd-alt-shift` + letter (no `fn`, no clashes with Zed defaults):

| Action | Key |
|---|---|
| Run (restarts it if it is already running) | `cmd-alt-shift-R` |
| Watch (hot reload) | `cmd-alt-shift-W` |
| Stop | `cmd-alt-shift-X` |
| Build | `cmd-alt-shift-B` |

All tasks are also available from `task: spawn` (`cmd-shift-r`).

> Debugging uses the same port as `dotnet: run`. Stop the running app (`cmd-alt-shift-X`) before pressing `F5`.

### Which project is used?

Starting at the directory of the open file, the nearest `*.csproj` / `*.fsproj` upwards wins. If there is none,
the workspace is searched and an executable project (Web / Worker / `OutputType` Exe) is preferred. Set the
`DOTNET_PROJECT` environment variable on a task to force one.

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

Supported options: `program`, `args`, `cwd`, `env`, `stopAtEntry`, `justMyCode`, `enableStepFiltering`,
`processId` (see `debug_adapter_schemas/netcoredbg.json`).

## Development

```sh
cargo build --release --target wasm32-wasip2
```

After changing the Rust code, use **Rebuild** on the extension in `zed: extensions`. Logs: `zed: open log`.

## License

MIT. netcoredbg is © Samsung Electronics, MIT licensed, and is downloaded from its
[GitHub releases](https://github.com/Samsung/netcoredbg/releases).
