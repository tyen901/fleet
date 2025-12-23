Problem
-------
Some upstream addon repositories do not publish a `manifest.json` inside each addon folder — they only provide the legacy `mod.srf` file (often with a UTF-8 BOM and addon ids that start with `@`). Our previous HTTP remote implementation assumed `manifest.json` existed and tried it first, which caused failures when the JSON manifest was missing.

Root cause
----------
- `url::Url::join` can interpret certain characters (notably `@`) as userinfo when given an unencoded path. Passing raw mod ids like `@cba_a3` could break URL handling.
- Code preferred `manifest.json` over `mod.srf`, but many real-world hosts only provide `mod.srf`.
- Some `mod.srf` files include a UTF-8 BOM which made the parser fail when not stripped before parsing.
