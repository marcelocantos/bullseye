# Startup configuration

Bullseye automatically reads `$XDG_CONFIG_HOME/bullseye/config.yaml`, falling
back to `~/.config/bullseye/config.yaml` when `XDG_CONFIG_HOME` is unset or
relative. Put persistent choices in this file. The minimal T3 configuration is:

```yaml
t3_plugin: true
```

Then start with `bullseye serve`, or run `supervisor/install.sh` for the
supervised service. The installed binary must support configuration files;
older releases only understand the environment and flag mechanisms. Upgrade
to v0.61.0 or later before using the updated supervisor installer, which
requires `serve --print-addr`.

Precedence, lowest to highest: **built-in defaults < YAML file < environment
< command line**. Settings are read once at startup; restart after edits.

| YAML key | Built-in default | Environment override | CLI override |
|---|---|---|---|
| `addr` | `127.0.0.1:18743` | `BULLSEYE_ADDR` | `--addr HOST:PORT` |
| `t3_plugin` | `false` | `BULLSEYE_T3_PLUGIN` | `--t3-plugin` / `--no-t3-plugin` |
| `data_dir` | `~/.local/share/bullseye` | `BULLSEYE_DATA_DIR` | `--data-dir PATH` |
| `default_location` | unset; creation asks | `BULLSEYE_DEFAULT_LOCATION` | `--default-location in_repo\|external` |
| `commit_mode` | `false` | `BULLSEYE_COMMIT` | `--commit-mode` / `--no-commit-mode` |
| `integration_ref` | `refs/heads/master` | `BULLSEYE_INTEGRATION_REF` | `--integration-ref REF` |
| `cache_ttl_secs` | `300` | `BULLSEYE_CACHE_TTL_SECS` | `--cache-ttl-secs N` |

YAML booleans are `true`/`false`. Environment booleans additionally accept
`1`/`0`, `yes`/`no`, and `on`/`off`, ignoring case and surrounding whitespace.
An explicitly empty environment boolean is false. CLI also accepts
`--t3-plugin=false` and `--commit-mode=false`: explicit false disables a
file/environment setting. Cache TTL accepts zero to disable caching.

Global startup flags go before the subcommand; `serve` also accepts them
after its name. Other commands' flags and values are never scanned as startup
options. Examples:

```sh
bullseye serve
BULLSEYE_T3_PLUGIN=0 bullseye serve
bullseye serve --no-t3-plugin
bullseye --data-dir /tmp/my-ledgers query --view frontier
bullseye serve --addr 127.0.0.1:18851 --print-addr
```

`--print-addr` prints the effective listener address and exits without binding
a port, starting a daemon or publishing a manifest. It uses the startup
resolver; the installer uses it to identify the listener it will manage.

## Selecting the file and paths

`--config FILE` selects a file, overriding `BULLSEYE_CONFIG`. Missing default
files preserve existing behavior; missing explicit files are errors. An empty
file means defaults. Malformed YAML, unknown keys, read failures and invalid
effective values prevent startup with diagnostics naming the file or setting.
`--no-config` bypasses default files and `BULLSEYE_CONFIG`, retaining env and
CLI overrides. It conflicts with an explicit `--config`. Help and version
remain available with a broken file.

Relative file selections resolve from the working directory. Relative
`data_dir` values inside YAML resolve from that file's directory; env/CLI
data directories resolve from the working directory. `~` and `~/...` use
`HOME`; shell `$VARIABLE` substitution inside YAML is not performed.

The location default only affects creating new ledgers; existing discovery
is unchanged. T3's loopback restriction applies regardless of the source of
its configuration. Process settings belong here; targets stay in `bullseye.yaml`.

## Supervisor

The launcher invokes `bullseye serve` and lets it read the user's file. It
does not inject a default address or force the plugin off. The installer
persists explicit `BULLSEYE_ADDR` and `BULLSEYE_T3_PLUGIN` overrides in its ini,
including false. Re-render without these overrides to restore file control.
Legacy `BULLSEYE_PORT` remains an explicit loopback-address override.
Invalid configuration aborts installation before any listener is stopped.

The daemon must have the intended `HOME` and, when customized,
`XDG_CONFIG_HOME`/`BULLSEYE_CONFIG` in its supervised environment. The
installer's environment does not replace supervisord's own environment for
custom file selection. No personal file is created or rewritten automatically.

Verification runs an isolated real daemon and CLI against scratch HOME,
ports and ledgers: file-driven binding, plugin probe/discovery, MCP creation,
CLI reads, overrides, false toggles and supervisor rendering. Existing
executable tests use `--no-config` to avoid personal configuration.
