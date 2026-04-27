# stamp

Manage the on-disk content store used by `freshcheck`.

## Subcommands

```
stamp record [OPTIONS] FILE...
stamp query  [OPTIONS] FILE...
stamp diff   [OPTIONS] FILE...
stamp gc     [OPTIONS]
```

| Subcommand | Reads file? | Reads store? | Writes store? |
| ---------- | ----------- | ------------ | ------------- |
| `record`   | yes | no  | yes |
| `query`    | no  | yes | no  |
| `diff`     | yes | yes | no  |
| `gc`       | no  | yes | yes |

## Store layout

```
.lu-store/
└── <bucket-byte-hex>/
    └── <fnv1a-rest-hex>.json
```

- Bucket byte: high byte of FNV-1a-64(path).
- File contents: a JSON object keyed by method name:

  ```json
  {
    "path": "src/main.c",
    "size": 1234,
    "blake3": "…",
    "crc32": "…",
    "timestamp": 1712345678
  }
  ```

Multiple methods coexist in the same file. Recording a new method
merges; recording the same method again overwrites.

## Options

| Flag | Notes |
| ---- | ----- |
| `--method=`        | repeatable; same enum as `freshcheck` |
| `--hash-algo=`     | as above |
| `--checksum-algo=` | as above |
| `--store=`         | default `.lu-store/` |
| `--format=`        | only meaningful for `query` and `diff` |

## Exit codes

- `record`, `gc`: 0 on success; 2 on error.
- `query`: 0 if every requested file has at least one signature; 1 if
  any requested file is absent from the store; 2 on error.
- `diff`: 0 if all queried files match their recorded signatures; 1 if
  any file differs; 2 on error or missing record.
