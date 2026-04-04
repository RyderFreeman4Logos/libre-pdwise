# Spike: Termux Clipboard Behavior

**Date**: 2026-04-04  
**Branch**: feat/implement-tiny  
**Scope**: Documentation research only. No local or device-side command execution was run for this spike.

## Summary

For the standard Termux stack (`termux-app` + `Termux:API` add-on + `termux-api` package):

- `termux-clipboard-set` supports both command-line arguments and `stdin`.
- `termux-clipboard-get` writes clipboard text to `stdout`.
- Successful command execution exits with `0`.
- CLI misuse in the shell wrappers exits with `1`.
- `TERMUX_VERSION` is a real Termux-provided environment variable, but it is **not** sufficient by itself to decide whether clipboard commands are available.
- For clipboard data, `stdin` is the safer input path than command-line arguments because argv can be exposed through `/proc/<pid>/cmdline`.
- In standard official Termux installs, the `Termux:API` app and the `termux-api` package are both required.

There is one important exception:

- The Google Play Termux fork has built-in support for `termux-clipboard-*`, so those builds do **not** require a separate `Termux:API` app for clipboard operations.

## `termux-clipboard-set` Behavior

### Usage mode

The upstream wrapper script documents:

```sh
Usage: termux-clipboard-set [text]
```

Its help text says the clipboard text is "either supplied as arguments or read from stdin if no arguments are given."

The implementation confirms this:

- if no positional arguments are passed, it runs the helper directly and reads from `stdin`;
- otherwise it does `echo -n "$@" | $CMD`.

### Practical conclusion

`termux-clipboard-set` supports **both**:

- `printf '%s' "hello" | termux-clipboard-set`
- `termux-clipboard-set "hello"`

But `stdin` is the better default for application code.

Reasons:

- security: the text does not appear in argv;
- fidelity: argument mode joins multiple arguments with spaces via `echo -n "$@"`;
- streaming: `stdin` naturally handles multiline content better.

### Recommended usage

```sh
printf '%s' "$text" | termux-clipboard-set
```

Avoid:

```sh
termux-clipboard-set "$text"
```

when `$text` may contain secrets or large/multiline content.

## `termux-clipboard-get` Behavior

### Usage mode

The upstream wrapper script documents:

```sh
Usage: termux-clipboard-get
```

It rejects extra positional arguments:

```sh
termux-clipboard-get: too many arguments
```

and then runs:

```sh
@TERMUX_PREFIX@/libexec/termux-api Clipboard
```

The official `termux-api` README says the helper binary forwards API-class output to the `stdout` of `termux-api`.

### Practical conclusion

`termux-clipboard-get` writes clipboard text to `stdout` on success, so shell capture patterns like these are appropriate:

```sh
termux-clipboard-get
value="$(termux-clipboard-get)"
termux-clipboard-get > file.txt
```

Diagnostics still go to `stderr` when the wrapper/helper reports an error.

## Exit Code Behavior

## Stable conclusions

- success path: `0`
- wrapper CLI misuse: `1`

The shell wrappers explicitly return `1` for invalid options, and `termux-clipboard-get` also returns `1` for extra positional arguments.

The helper executable `termux-api-broadcast` returns `0` after `run_api_command()` completes without needing a callback file descriptor.

## Important nuance

Runtime failure is **not** documented as a single stable non-zero code.

Source review shows several lower-level helper paths that call `exit(1)` or return `-1`, but the top-level executable also has a path that returns `0` after `run_api_command()` returns `-1`.

Inference:

- you can rely on `0` meaning "normal success path";
- you can rely on `1` for wrapper argument misuse;
- you should **not** assume every runtime/API failure has one single stable non-zero exit code;
- in defensive code, check both exit status and whether expected `stdout` content was actually produced.

For `termux-clipboard-get`, that means an empty/unexpected result should be treated carefully even if the exit code is not informative enough.

## Environment Detection

## Is `TERMUX_VERSION` real?

Yes.

The official Termux app source defines `ENV_TERMUX_VERSION`, and the Termux execution-environment docs point to that shell-environment code as the source of exported Termux variables.

## Is `TERMUX_VERSION` enough?

No.

`TERMUX_VERSION` only tells you that the shell environment came from the Termux app. It does **not** guarantee:

- the `termux-api` package is installed;
- `termux-clipboard-set` and `termux-clipboard-get` are present on `PATH`;
- the separate `Termux:API` add-on app is installed in standard official Termux setups.

## Recommended detection strategy

For clipboard capability, prefer **capability detection** over pure environment detection:

```sh
command -v termux-clipboard-set >/dev/null 2>&1 &&
command -v termux-clipboard-get >/dev/null 2>&1
```

Optionally also check package presence in standard official Termux:

```sh
dpkg -s termux-api >/dev/null 2>&1
```

If you also want to verify "this is a Termux-started shell", the more structured Termux-specific variables are better than guessing from general Android paths:

- `TERMUX_VERSION`
- `TERMUX_APP__PACKAGE_NAME`
- `TERMUX__PREFIX` on newer app versions

### Suggested policy for libre-pdwise

1. Detect clipboard command availability with `command -v`.
2. Use `TERMUX_VERSION` or `TERMUX_APP__PACKAGE_NAME` only as supplemental context.
3. Do not gate clipboard support on `TERMUX_VERSION` alone.

## Security Notes

Do **not** pass sensitive clipboard content as command-line arguments unless there is no alternative.

The Linux `/proc/<pid>/cmdline` interface exposes a running process's command-line arguments. The man page states that this read-only file holds the complete command line for the process.

That means content passed like this:

```sh
termux-clipboard-set "$secret"
```

may be exposed through process inspection while the command is running.

Safer pattern:

```sh
printf '%s' "$secret" | termux-clipboard-set
```

Additional notes:

- avoid shell history leakage from inline literals;
- prefer `printf '%s'` over `echo` for exact text handling;
- avoid assuming argument mode preserves exact structure for multiline content.

## Required Packages

## Standard official Termux (GitHub/F-Droid)

Clipboard support requires both:

1. the `Termux:API` Android add-on app
2. the `termux-api` package inside Termux

The Termux wiki explicitly says the add-on is required for API implementations to function and also says:

```sh
pkg install termux-api
```

The main `termux-app` README also lists `Termux:API` as an optional plugin app.

## Google Play Termux fork

This is different.

The Google Play Termux fork release notes state that `termux-clipboard-*` is built into the main app and does not require `Termux:API`.

So if libre-pdwise targets the standard official Termux distribution, document the requirement as:

- install `Termux:API`
- run `pkg install termux-api`

If later Google Play fork support matters, document it as a separate compatibility branch instead of weakening the main requirement.

## Recommended Documentation Snippet for Implementation Work

```md
On standard Termux installs, clipboard support requires the `Termux:API` app and the `termux-api` package. Use `termux-clipboard-set` via stdin (`printf '%s' "$text" | termux-clipboard-set`) rather than command-line arguments to avoid argv exposure and preserve text more reliably. Read clipboard text with `termux-clipboard-get`, which writes to stdout.
```

## Sources

- `termux-clipboard-set` wrapper script: https://github.com/termux/termux-api-package/blob/master/scripts/termux-clipboard-set.in
- `termux-clipboard-get` wrapper script: https://github.com/termux/termux-api-package/blob/master/scripts/termux-clipboard-get.in
- `termux-api` README: https://github.com/termux/termux-api/blob/master/README.md
- `termux-api-broadcast.c`: https://github.com/termux/termux-api-package/blob/master/termux-api-broadcast.c
- `termux-api.c`: https://github.com/termux/termux-api-package/blob/master/termux-api.c
- `CMakeLists.txt` install layout: https://github.com/termux/termux-api-package/blob/master/CMakeLists.txt
- Termux execution environment wiki: https://github.com/termux/termux-packages/wiki/Termux-execution-environment
- Termux app shell environment source: https://raw.githubusercontent.com/termux/termux-app/master/termux-shared/src/main/java/com/termux/shared/termux/shell/command/environment/TermuxAppShellEnvironment.java
- Termux app README: https://github.com/termux/termux-app
- Termux wiki page for `Termux:API`: https://wiki.termux.com/wiki/Termux%3AAPI?lang=fr
- Linux procfs man page for `/proc/<pid>/cmdline`: https://man7.org/linux/man-pages/man5/proc_pid_cmdline.5.html
