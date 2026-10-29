#!/bin/sh
# Helper behind the optional Zed tasks installed by install.sh.
#
#   dotnet-zed.sh <run|watch|build|stop> <worktree-root> [args...]
#
# The project is resolved from the current directory (the task cwd, i.e. the
# directory of the open file), walking up until a *.csproj is found. If there
# is none, the worktree is searched and an executable project is preferred.
# Set DOTNET_PROJECT to force a specific project.

action="$1"
root="${2:-$PWD}"
[ $# -ge 2 ] && shift 2 || shift $#

find_project() {
    if [ -n "$DOTNET_PROJECT" ]; then
        echo "$DOTNET_PROJECT"
        return
    fi

    dir="$PWD"
    while :; do
        for f in "$dir"/*.csproj "$dir"/*.fsproj; do
            [ -f "$f" ] && { echo "$f"; return; }
        done
        [ "$dir" = "$root" ] || [ "$dir" = "/" ] && break
        dir=$(dirname "$dir")
    done

    candidates=$(find "$root" \( -name bin -o -name obj -o -name node_modules -o -name .git \) -prune \
        -o \( -name '*.csproj' -o -name '*.fsproj' \) -print 2>/dev/null | sort)
    for f in $candidates; do
        if grep -qE 'Microsoft\.NET\.Sdk\.(Web|Worker|BlazorWebAssembly)|<OutputType>(Win)?Exe' "$f"; then
            echo "$f"
            return
        fi
    done
    echo "$candidates" | head -n 1
}

project=$(find_project)
if [ -z "$project" ] || [ ! -f "$project" ]; then
    echo "dotnet-zed: no .csproj/.fsproj found from $PWD (root: $root)" >&2
    exit 1
fi
project_dir=$(dirname "$project")

stop_project() {
    # `dotnet run --project <csproj>` / `dotnet watch` and the app itself (bin/...).
    pkill -f -- "$project" 2>/dev/null
    pkill -f -- "$project_dir/bin/" 2>/dev/null && sleep 1
    return 0
}

case "$action" in
    run|restart)
        # Running again restarts: stop whatever instance of this project is already up.
        stop_project
        echo "▶ dotnet run  ($project)"
        cd "$project_dir" && exec dotnet run --project "$project" ${1:+--} "$@"
        ;;
    watch)
        stop_project
        echo "▶ dotnet watch  ($project)"
        cd "$project_dir" && exec dotnet watch --project "$project" run ${1:+--} "$@"
        ;;
    build)
        # Extra args are MSBuild args (the debugger passes -getResultOutputFile etc.).
        echo "▶ dotnet build  ($project)"
        exec dotnet build "$project" "$@"
        ;;
    stop)
        stop_project
        echo "■ stopped ($project)"
        ;;
    *)
        echo "usage: dotnet-zed.sh <run|watch|build|stop> <worktree-root> [args...]" >&2
        exit 2
        ;;
esac
