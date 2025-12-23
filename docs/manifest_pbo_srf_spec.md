# Fleet / Nimble Repository File Format Specification (manifest.json, mod.srf, PBO)

This document defines the **exact on-disk / over-HTTP shapes** and **checksum algorithms** used by the Swifty-compatible ecosystem (as implemented in the provided Fleet and Nimble code). The goal is to eliminate future reverse‑engineering and ensure all implementations converge on the same behavior.

## Conventions

### Encoding
- **JSON files** (`repo.json`, `manifest.json`, `mod.srf` JSON) are UTF‑8.
- Producers MAY include a UTF‑8 BOM (`EF BB BF`) at the start of a JSON file; consumers MUST strip it before JSON parsing.

### Paths
- `Path` fields are **relative paths** within a mod root (never absolute).
- Consumers MUST normalize path separators: **replace `\` with `/`** before validation and before checksum calculations that involve paths.
- When computing mod checksums, paths are lowercased using **ASCII lowercase** after normalization (see below).

### MD5 representation
- All digests are **MD5** (16 bytes).
- In JSON/text, digests are represented as **32 hex characters**.
- Producers SHOULD emit **uppercase hex**; consumers MUST accept either case, but checksum algorithms below require uppercase hex when concatenating.

---

## 1) `repo.json` (repository descriptor)

Although the primary focus is manifest/PBO/SRF, `repo.json` is included because it is the entry point for selecting mods and for optional basic auth and server metadata.

### File name and location
- At repository root: `repo.json`

### JSON shape (camelCase)
```json
{
  "repoName": "ExampleRepo",
  "checksum": "opaque-string",
  "requiredMods": [
    { "modName": "@ace", "checkSum": "787662722D70C36DF28CD1D5EE8D8E86", "enabled": true }
  ],
  "optionalMods": [
    { "modName": "@cba_a3", "checkSum": "44C1B8021822F80E1E560689D2AAB0BF", "enabled": false }
  ],
  "clientParameters": "-noLauncher",
  "repoBasicAuthentication": { "username": "user", "password": "pass" },
  "version": "1",
  "servers": [
    { "name": "Main", "address": "127.0.0.1", "port": 2302, "password": "", "battleEye": true }
  ]
}
```

### Field types
- `repoName`: string
- `checksum`: string (not MD5‑semantics; treated as opaque in Nimble)
- `requiredMods` / `optionalMods`: arrays of:
  - `modName`: string (mod directory name, typically begins with `@`)
  - `checkSum`: **MD5 hex string** (32 chars)
  - `enabled`: boolean
- `clientParameters`: string
- `repoBasicAuthentication`: optional object `{ username: string, password: string }`
- `version`: string
- `servers`: array of:
  - `name`: string
  - `address`: string (IP or hostname)
  - `port`: number **or** string containing a number (u16)
  - `password`: string
  - `battleEye`: boolean

---

## 2) `manifest.json` (mod manifest)

### File name and location
- Under each mod directory:
  - `{mod_id}/manifest.json`

### Purpose
`manifest.json` describes the expected files for a mod, including part boundaries and per‑part checksums.

### JSON shape (PascalCase)
```json
{
  "Name": "@ace",
  "Checksum": "787662722D70C36DF28CD1D5EE8D8E86",
  "Files": [
    {
      "Path": "addons/ace_advanced_ballistics.pbo",
      "Length": 1234567,
      "Checksum": "A1B2C3D4E5F60718293A4B5C6D7E8F90",
      "Parts": [
        { "Start": 0, "Length": 4096, "Checksum": "..." },
        { "Start": 4096, "Length": 8192, "Checksum": "..." }
      ]
    }
  ]
}
```

### Field types
Top-level:
- `Name`: string (the mod identifier; used as the remote directory name)
- `Checksum`: MD5 hex string (32 chars) — **the mod checksum**, computed as described in §6
- `Files`: array of file entries

File entry (`Files[]`):
- `Path`: string (relative path within the mod)
- `Length`: integer (u64)
- `Checksum`: MD5 hex string — **the file checksum**, computed as described in §6
- `Parts`: array of parts

Part entry (`Parts[]`):
- `Start`: integer (u64) — byte offset from start of the file
- `Length`: integer (u64) — number of bytes
- `Checksum`: MD5 hex string — MD5 of the part bytes in that range

### Required invariants (Swifty-compatible)
- `Files[].Path` MUST be a relative path and MUST NOT contain `..`, a leading `/`, a Windows drive prefix (`C:`), or NUL bytes.
- `Parts` MUST be sorted by `Start` (ascending).
- For non-empty files, `Parts` MUST cover the entire file:
  - contiguous, no gaps, no overlaps
  - first part starts at `0`
  - sum of all part lengths equals `Length`
- For empty files (`Length == 0`), `Parts` MUST be an empty array.

### Compatibility notes
- Some real-world manifests have been observed to include **zero-length parts** (`Parts[].Length == 0`). These parts are semantically no-ops.
  - Producers MUST NOT emit zero-length parts.
  - Consumers SHOULD ignore/drop zero-length parts before validating contiguity and coverage.

---

## 3) `mod.srf` (SRF manifest)

`mod.srf` exists in two formats:
1) a **JSON** format (the most common modern format)
2) a **legacy text** format (older Swifty output)

Fleet treats `.srf` fixtures as JSON despite the extension; Nimble supports both JSON and legacy formats.

### 3.1 JSON `mod.srf`

#### File name and location
- Under each mod directory:
  - `{mod_id}/mod.srf`

#### JSON shape
The JSON `mod.srf` format is structurally aligned with `manifest.json` (PascalCase), with *additional optional fields* that are ignored by tolerant parsers.

```json
{
  "Name": "@ace",
  "Checksum": "787662722D70C36DF28CD1D5EE8D8E86",
  "Files": [
    {
      "Path": "addons/ace_advanced_ballistics.pbo",
      "Length": 1234567,
      "Checksum": "A1B2C3D4E5F60718293A4B5C6D7E8F90",
      "Type": "SwiftyPboFile",
      "Parts": [
        { "Path": "$$HEADER$$", "Start": 0, "Length": 4096, "Checksum": "..." },
        { "Path": "some_inner_entry.sqf", "Start": 4096, "Length": 8192, "Checksum": "..." },
        { "Path": "$$END$$", "Start": 12288, "Length": 123, "Checksum": "..." }
      ]
    },
    {
      "Path": "keys/ace_key.bikey",
      "Length": 2048,
      "Checksum": "....",
      "Type": "SwiftyFile",
      "Parts": [
        { "Path": "ace_key.bikey_5000000", "Start": 0, "Length": 2048, "Checksum": "..." }
      ]
    }
  ]
}
```

#### Notes
- `Files[].Type` (string) is OPTIONAL:
  - `"SwiftyFile"` or `"SwiftyPboFile"`
- `Parts[].Path` (string) is OPTIONAL; Swifty uses:
  - For PBO files: `$$HEADER$$`, each PBO entry filename, then `$$END$$`
  - For regular files: a generated label (e.g., `{filename}_{pos}`)
- Consumers MUST NOT rely on `Type` or `Parts[].Path` for correctness; **only offsets/lengths and checksums are authoritative**.

All checksum and invariant rules from `manifest.json` (§2) apply to JSON `mod.srf`.

### 3.2 Legacy text `mod.srf`

#### Detection
- Legacy files begin with ASCII `"ADDON"` at the start of the file (Nimble checks the first 5 bytes).

#### Format
The legacy SRF is a line-oriented, colon-delimited format. It is stateful:
- First line describes the addon and includes the number of files.
- Then each file header line is followed by `part_count` part lines.

##### Addon header line
```
ADDON:{name}:{file_count}:{checksum}
```
- `name`: string (e.g., `@lambs_danger`)
- `file_count`: u32 (decimal)
- `checksum`: 32-char MD5 hex digest

##### File header line (repeated `file_count` times)
```
{TYPE}:{path}:{length}:{part_count}:{checksum}
```
- `TYPE`: `"PBO"` or `"FILE"`
- `path`: relative path string (as stored; may contain backslashes)
- `length`: u64 (decimal)
- `part_count`: u32 (decimal)
- `checksum`: MD5 hex digest string (uppercase expected)

##### Part line (repeated `part_count` times immediately after a file header)
```
{path}:{start}:{length}:{checksum}
```
- `path`: string label (may be `$$HEADER$$`, `$$END$$`, or any other label)
- `start`: u64 (decimal)
- `length`: u64 (decimal)
- `checksum`: MD5 hex digest string

#### BOM handling
Producers have historically emitted BOMs; consumers MUST strip a leading UTF‑8 BOM **before** legacy detection and parsing.

---

## 4) `.pbo` (Arma PBO file) — subset required for Swifty-compatible hashing

This section specifies the subset of the PBO binary format required to reproduce Swifty/Nimble checksum partitioning. It intentionally focuses on header parsing sufficient to compute `header_len` and entry `data_size` values.

### 4.1 Header entry record

A PBO begins with a sequence of header entries. Each entry is:

1. `filename`: NUL-terminated byte string (C string). The empty string terminates the header table.
2. Five little-endian u32 values:
   - `type`: u32
   - `original_size`: u32
   - `offset`: u32
   - `timestamp`: u32
   - `data_size`: u32

The header table ends with a terminator entry:
- `filename == ""` (empty C string)
- `type == 0` (commonly called `None`)

### 4.2 Entry type values used by Swifty/Nimble
Swifty/Nimble recognize the following `type` values (FourCC encoded in little-endian u32):

| Symbol | FourCC bytes | u32 hex |
|---|---:|---:|
| `Vers` | `V e r s` | `0x56657273` |
| `Cprs` | `C p r s` | `0x43707273` |
| `Enco` | `E n c r` | `0x456e6372` |
| `None` | `\0\0\0\0` | `0x00000000` |

### 4.3 Extensions map following a `Vers` entry
If an entry has `type == Vers`, then immediately after that entry record the file contains an **extensions map**:

- repeated pairs of:
  1) key: NUL-terminated string  
  2) value: NUL-terminated string
- terminated by an empty key string.

Swifty/Nimble parse this map to advance the stream position correctly; they do not require any specific keys/values for hashing.

### 4.4 `header_len` definition
`header_len` is the file position (byte offset from start) **immediately after**:
- the header entries table terminator entry, and
- any extensions map associated with a `Vers` entry.

Implementations should compute it as the stream position after parsing (as Nimble does).

---

## 5) Swifty-compatible part partitioning for PBO files

When generating SRF/manifest parts for a `.pbo`, Swifty-compatible tools create parts as follows.

### Inputs
- Parsed `header_len`
- Parsed `entries[]` (in header order, including the `Vers` entry if present)
- File length `file_len` (seek to end)

### Algorithm (Swifty-compatible mode)
1) **Header part**
- Part #0:
  - `Start = 0`
  - `Length = header_len`
  - `Checksum = MD5(bytes[0..header_len])`
  - (optional) `Path = "$$HEADER$$"`

2) **Skip the first entry**
- The **first** header entry (index 0) is skipped when creating data parts.
- Compatibility requirement: the skipped first entry MUST have `data_size == 0`. If it does not, the file is considered incompatible with Swifty layout.

3) **Data parts for remaining entries**
- Maintain a running `offset`, initially `header_len`.
- For each header entry `entries[i]` where `i >= 1`:
  - Let `len = entries[i].data_size`.
  - If `len == 0`, emit no part and do not advance `offset`.
  - Else emit a part:
    - `Start = offset`
    - `Length = len`
    - `Checksum = MD5(bytes[offset..offset+len])`
    - (optional) `Path = entries[i].filename`
  - Advance `offset += len`.

**Important:** Swifty-compatible tools do **not** use the stored `offset` field from the header records for partitioning; they treat the data region as a sequential stream starting at `header_len` and consume `data_size` bytes in header order.

4) **End part**
- After consuming all entry data sizes, let `remaining = file_len - offset`.
- If `remaining > 0`, emit one final part:
  - `Start = offset`
  - `Length = remaining`
  - `Checksum = MD5(bytes[offset..file_len])`
  - (optional) `Path = "$$END$$"`
- The emitted parts MUST cover the entire file length exactly.

### Non-Swifty (spec) mode
Some tools may choose to **not** skip the first entry when creating parts (include all entries in order). This is not the Swifty-compatible mode and should only be used if explicitly negotiated.

---

## 6) Checksum algorithms

### 6.1 Part checksum
- For a part defined by `(Start, Length)`, the part checksum is:
  - `MD5(file_bytes[Start .. Start+Length])`
- Encoded as uppercase hex when serialized.

### 6.2 File checksum from parts
Given a file’s `Parts[]` in order (by ascending `Start`):

1. For each part, take `part.Checksum` as an **uppercase** 32‑char hex string.
2. Concatenate these strings with **no separators**.
3. Compute `MD5( UTF-8 bytes of the concatenated string )`.

That MD5 is the `Files[].Checksum`.

### 6.3 Mod checksum from files
Given all files in a mod:

1. Normalize each file path:
   - `norm_path = Path.replace("\\", "/")`
2. Sort files by `norm_path` in **ASCII case-insensitive** order (equivalently: sort by `norm_path.to_ascii_lowercase()`).
3. Build the checksum input by concatenating, for each file in sorted order:
   - `Files[].Checksum` as uppercase hex string (32 chars)
   - `norm_path.to_ascii_lowercase()` (ASCII lowercase)
4. Compute `MD5( UTF-8 bytes of the concatenated sequence )`.

That MD5 is the top-level `Checksum` in `manifest.json` / `mod.srf`.

---

## 7) Validation checklist (recommended)

A consumer/validator should reject a manifest/SRF if any of the following are true:

- A file path is unsafe (absolute, contains `..`, contains NUL, or has a Windows drive prefix).
- `Length` is inconsistent with the parts:
  - parts sum != length (for non-empty files), or
  - first part does not start at 0, or
  - parts overlap, are unsorted, or contain gaps (for Swifty-compatible manifests).
- Parts contain `Length == 0` entries and, after ignoring/dropping those entries, the remaining parts still violate the invariants above.
- Any part range exceeds `Length` (`Start + Length > file.Length`).
- For PBO Swifty layout generation: the skipped first entry does not have `data_size == 0`.

---

## 8) Compatibility notes for implementers

- Treat `Type` and `Parts[].Path` as informational only; do not make correctness depend on them.
- If a manifest contains any `Parts[].Length == 0`, treat those as no-ops and ignore/drop them before validating coverage/contiguity.
- Always normalize path separators before:
  - comparisons
  - sorting
  - checksum input that includes paths
- Strip UTF‑8 BOMs before parsing JSON or legacy SRF text.
- Ensure all checksum string concatenations use **uppercase hex digests** for the part/file checksums.
