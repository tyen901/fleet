# Fleet / Nimble Repository Formats

This specification defines the repository formats required for Fleet/Nimble interoperability **without `mod_manifest.json`**.
It is derived from the real fixtures:

- `example_repo.json`
- `example_mod.srf` (JSON SRF, BOM-prefixed)
- `legacy_text_mod.srf` (legacy text SRF, `ADDON:` format)
- `example_pbo.pbo` (used to verify PBO partitioning structure)

## 0. Terms and conventions

### 0.1 Encoding and BOM
- JSON payloads are UTF-8.
- A UTF-8 BOM (`EF BB BF`) **may** prefix JSON files (observed in `example_mod.srf`).
  - Consumers **must** strip the BOM before JSON parsing.
- Legacy text SRF is line-oriented UTF-8 (ASCII subset) and starts with `ADDON:`.

### 0.2 Hex digests
- In SRF files:
  - Mod, file, and part checksums are **MD5**, serialized as **32 hex characters**.
  - Producers commonly emit **uppercase**; consumers should accept either case when parsing.
- In `repo.json`:
  - `requiredMods[].checkSum` / `optionalMods[].checkSum` are **MD5** (32 hex).
  - Other checksum-like fields (e.g. `checksum`, `iconImageChecksum`, `repoImageChecksum`) are **opaque strings**
    and must not be assumed to be MD5 (real fixture uses 40 hex characters).

### 0.3 Paths
- All file paths inside a mod are **relative** paths.
- Normalization used in checksum algorithms:
  1. Replace backslashes: `\` -> `/`
  2. ASCII lowercase: `to_ascii_lowercase()`

Safety constraints for paths (recommended validation):
- Must not be absolute (no leading `/`, no drive prefixes like `C:`)
- Must not contain parent traversal segments (`..`)
- Must not contain NUL bytes

---

## 1. Repository root file: `repo.json`

### 1.1 Location
- Repository root: `repo.json`

### 1.2 JSON shape (camelCase; `checkSum` spelling)
The real fixture is consistent with the following shape:

```json
{
  "repoName": "modpack_test",
  "checksum": "64A98E00EB1A04D3791E463EC87F40E810D9106A",
  "requiredMods": [
    { "modName": "@cba_a3", "checkSum": "6BF82A0530E8E60D1D24EFD07E0B0FC4", "enabled": true }
  ],
  "optionalMods": [],
  "clientParameters": "-skipIntro",
  "repoBasicAuthentication": { "username": "userName", "password": "test" },
  "version": "3.2.0.0",
  "servers": [
    { "name": "My super server", "address": "test.server", "port": "3000", "password": "password", "battleEye": false }
  ]
}
```

### 1.3 Compatibility rules
- Unknown fields must be ignored for forwards compatibility.
- `servers[].port` must accept both:
  - JSON number, or
  - numeric string (observed: `"3000"`).
- `servers[].address` is a string and may be a hostname (observed: `"test.server"`).

---

## 2. Mod SRF file: `mod.srf`

### 2.1 Location
- For a mod identified by `{mod_id}`: `{mod_id}/mod.srf`

### 2.2 Format detection
After stripping an optional UTF-8 BOM:
- If the first non-BOM byte is `{`, parse as **JSON SRF** (Section 2.3).
- Else if the file starts with `ADDON:`, parse as **legacy text SRF** (Section 2.4).
- Otherwise: unsupported SRF format.

### 2.3 JSON SRF (PascalCase keys)

#### 2.3.1 Top-level object
JSON SRF uses PascalCase field names:

```json
{
  "Name": "@acre2",
  "Checksum": "FEBE2EFFB7A464B2859752FE6A312E3B",
  "Files": [ ... ]
}
```

#### 2.3.2 File entries
Each element of `Files` has:

```json
{
  "Path": "addons\\acre_sys_zeus.pbo",
  "Length": 30835,
  "Checksum": "220C803A6EE5CE8F2CC1507306E96B5C",
  "Type": "SwiftyPboFile",
  "Parts": [ ... ]
}
```

Observed `Type` values in real fixture:
- `SwiftyFile`
- `SwiftyPboFile`

`Type` is informational; consumers must not depend on it for correctness.

#### 2.3.3 Part entries
Each element of `Parts` has:

```json
{
  "Path": "$$HEADER$$",
  "Length": 706,
  "Start": 0,
  "Checksum": "7D9BE7DEB043F2926E11B74054A97839"
}
```

- `Path` is informational/labeling (see Section 4).
- Only `Start`, `Length`, and `Checksum` are required for validation and downloads.

#### 2.3.4 Invariants (recommended validation)
For each file:
- Parts must be sorted by `Start` ascending.
- For `Length > 0`, parts must cover the file exactly:
  - first part `Start == 0`
  - contiguous: `next.Start == prev.Start + prev.Length`
  - final end equals `Length`
- For `Length == 0`, parts must be empty.

Unknown fields in JSON SRF should be ignored.

### 2.4 Legacy text SRF (`ADDON:` format)

Legacy SRF is line-oriented and colon-delimited.

#### 2.4.1 Addon header (first line)
```
ADDON:{name}:{file_count}:{mod_checksum}
```

Example (from fixture):
```
ADDON:@lambs_danger:19:44C1B8021822F80E1E560689D2AAB0BF
```

#### 2.4.2 File header lines
Then repeat `file_count` times:

```
{TYPE}:{path}:{length}:{part_count}:{file_checksum}
```

- `TYPE` observed in Nimble-compatible legacy SRF: `FILE` or `PBO`.
- `path` may contain backslashes.

#### 2.4.3 Part lines
Immediately following each file header are `part_count` lines:

```
{label}:{start}:{length}:{part_checksum}
```

- `label` is informational (see Section 4).
- `start` and `length` are decimal integers.

---

## 3. Remote file downloads

Given a repository base URL and a mod id:
- SRF: `GET {base}/{mod_id}/mod.srf`
- File bytes: `GET {base}/{mod_id}/{relative_path}`

### 3.1 Optional HTTP range requests
Some clients may use `Range: bytes={start}-{end}` to fetch part ranges.
If ranges are used, servers should respond with `206 Partial Content` for valid ranges.

(Clients that require ranges must detect support and fail gracefully when the server ignores ranges.)

---

## 4. Partitioning rules (how Parts are produced)

### 4.1 Non-PBO files (SwiftyFile)
Nimble-compatible partitioning for non-PBO files is fixed-size chunking:

- Iterate through the file from offset 0.
- Emit consecutive parts of up to **5,000,000 bytes** each (last part may be smaller).
- Each part checksum is MD5 over the raw bytes for that range.
- Part labels (`Parts[].Path` in JSON SRF / `{label}` in legacy SRF) are informational; a common pattern is:
  - `{filename}_{end_offset}` (e.g., `acre2_win64.dll_2242384`).

The real `example_mod.srf` fixture contains only files smaller than 5,000,000 bytes, so all `SwiftyFile` entries have a single part there; however the chunk size remains normative for compatibility.

### 4.2 PBO files (SwiftyPboFile)

PBO partitioning is **header + per-entry + tail**, derived from parsing the PBO header table.

#### 4.2.1 Header parsing essentials
A PBO begins with a sequence of entries. Each entry consists of:
1. `filename`: NUL-terminated string
2. 5x little-endian `u32`: `type`, `original_size`, `offset`, `timestamp`, `data_size`

The header table terminates at an entry where:
- `filename == ""` and `type == 0`.

If an entry has `type == 0x56657273` (`'Vers'`), it is followed by an extensions map:
- repeating `{key}\0{value}\0`
- terminated by empty key (`"\0"`)

Define:
- `header_len` = byte position immediately after finishing the terminating entry and any extensions maps encountered.

In the real `example_pbo.pbo` fixture:
- the first entry is a `Vers` entry with empty filename and `data_size == 0`,
- and `header_len` is non-zero.

#### 4.2.2 Swifty-compatible partition algorithm
Given `header_len`, the parsed entries in order, and `file_len`:

1. Emit header part:
   - `Start = 0`
   - `Length = header_len`
   - label `$$HEADER$$` (informational)

2. Set `offset = header_len`.

3. For each entry **after the first header entry** (i.e., entries starting at index 1):
   - Let `len = data_size`.
   - If `len > 0`:
     - Emit a part with:
       - `Start = offset`
       - `Length = len`
       - label = entry `filename` (informational)
   - Set `offset += len`.

4. If `offset < file_len`:
   - Emit a final tail part:
     - `Start = offset`
     - `Length = file_len - offset`
     - label `$$END$$` (informational)

Notes:
- The `offset` field stored in each header entry is **not** used for partitioning in this mode; only the sequential accumulation is used.
- Parts must cover the entire file exactly.

#### 4.2.3 Label conventions (observed)
For PBO parts in JSON SRF:
- First part label: `$$HEADER$$`
- Subsequent part labels: PBO entry filenames
- Final part label: `$$END$$`

---

## 5. Checksum algorithms (normative)

All checksum concatenations below operate over the **ASCII bytes** of the hex strings (no separators).

### 5.1 Part checksum
For each part `(Start, Length)`:
- `part_checksum = MD5(file_bytes[Start .. Start+Length])`
- Serialize as 32 hex characters (uppercase recommended).

### 5.2 File checksum from parts
Given parts in ascending `Start` order:
1. Convert each part checksum to its **uppercase** 32-hex string form.
2. Concatenate these strings with no separators.
3. `file_checksum = MD5(ASCII_bytes(concatenated_hex_strings))`

For an empty file with zero parts, the concatenation is empty and the result is `MD5("")`.

### 5.3 Mod checksum from files
Given all files in a mod:
1. For each file, compute `norm_path` = `Path` normalized (Section 0.3).
2. Sort files by `norm_path` ascending.
3. Build the hash input bytes by concatenating, for each file in sorted order:
   - `file_checksum` as **uppercase** 32-hex string (ASCII bytes)
   - `norm_path` as ASCII-lowercased UTF-8 bytes
4. `mod_checksum = MD5(concatenated_bytes)`

### 5.4 Verification rules
A consumer must reject an SRF if:
- a file has `Length > 0` but zero parts, or parts do not cover the file exactly
- derived `file_checksum` (from parts) does not equal the file’s declared checksum
- derived `mod_checksum` (from files) does not equal the SRF top-level checksum

---

## 6. Implementation checklist

- Strip UTF-8 BOM before JSON parsing and before SRF format detection.
- Detect legacy SRF by `ADDON:` prefix.
- Normalize paths for hashing (backslash→slash, lowercase).
- Ignore unknown fields in JSON (`repo.json` and JSON SRF).
- Treat non-MD5 checksum-like fields in `repo.json` as opaque strings.
