# Configuration

A TOML file supplies defaults for `--store`, `--listen`, `--no-open` and
`prune --age`, so a store kept outside the default location needs naming only
once, and a scheduled `cases prune --yes` needs no arguments. Precedence is
flag > environment variable > config file > built-in default.

The file is read from `$XDG_CONFIG_HOME/cases/config.toml`, falling back to
`~/.config/cases/config.toml`. Override the location with `--config PATH` or
the `CASES_CONFIG` environment variable. The file is optional.

```sh
cases config init    # write a commented template (refuses to overwrite; --force to replace)
cases config path    # print the file in use and whether it exists
cases config show    # print the defaults the environment and the file establish
```

| Key | Sets | Beaten by | Default |
| --- | --- | --- | --- |
| `store` | `--store` | `CASES_STORE` | `$XDG_DATA_HOME/cases/cases.db`, or `~/.local/share/cases/cases.db` |
| `listen` | `--listen` on `serve` and `service install` | nothing | `127.0.0.1:8765` |
| `no-open` | `--no-open` on `serve` | nothing | `false` (`true` or `false`, quoted or not) |
| `name` | `--as` on `answer`, `resume`, `serve` and `service install` | nothing | none: no actor is recorded |
| `prune-age` | `--age` on `prune` | nothing | `720h` |

```toml
store = "~/cases/work.db"
```

A leading `~` in `store` is expanded. `CASES_STORE` set to an empty string
counts as unset.

Values are strings, except `no-open`, which also takes an unquoted `true` or
`false`. An unknown key, a value of the wrong type, or malformed TOML is an
error that names the file. It stops every command except `cases config path`,
`cases config show` and `cases config init`, which are how you find out which
file is at fault.
