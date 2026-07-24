# Cubic Launcher — Launch-verification matrix: GREEN

**Result: API pass 69/69 green · Cache pass 69/69 green.**
Full sweep on native display (xvfb unavailable here), persistent workspace
`~/cubic-harness-data`, `--success-after 45 --timeout 900`. Machine-generated
table: `reports/matrix-report.md` (69 API rows + 69 cache rows, all ✅).

Negative fixtures count green when they fail as expected (`launch_failed`).
Positives count green only at in-world `running` (≈46 s wall each).

Gate per commit: `cargo test` 199 passed / 0 failed / 1 ignored, `cargo clippy
--lib --tests` 0 errors. Branch `fixes/security-audit-waves`, **not pushed**.

---

## Matrix composition (69 combos)

- 61 positives (must reach `running`), 8 negatives (must `launch_failed`).
- Negatives: `fabric@1.13.2`, `neoforge@{1.15.2,1.16.5,1.17.1,1.18.2,1.19.2,1.19.4}`,
  `forge@1.20.5`.

---

## Bucket-by-bucket: what was fixed (file:line)

### D — Matrix reclassifications (`chore(matrix)`, commits `c3e6f86`, `cc01bdb`)
- `matrix.py:38-39` — `1.6.4` added to STRATUM_A as positive vanilla combo
  (was a negative fixture). Full legacy support 1.6.4–1.12.2.
- `matrix.py:45-63` — removed `neoforge` from positive loaders of
  `{1.15.2,1.16.5,1.17.1,1.18.2,1.19.2}`.
- `matrix.py:85-96` — those five join `neoforge@1.19.4` as negative fixtures
  (NeoForge does not exist before 1.20.1).
- `matrix.py:55-56,98-99` — `forge@1.20.5` → negative fixture (**deviation**, see below).

### C — Java nearest-higher fallback (`feat(java)`, commit `df21b6e`)
Root cause: MC 1.17.x needs Java 16, which Adoptium no longer ships (EOL →
empty asset list → `fetch_latest_jre_package` returns `Ok(None)` → launch failed
pre-spawn on all 1.17.1 loaders). Verified live: `GET /v3/assets/latest/16/...`
returns `200` with `[]`.
- `java_runtime.rs:181-198` — `select_java_for_requirement` exact-match →
  nearest-higher (lowest installed major ≥ requirement).
- `java_runtime.rs:203-219` — new `select_exact_java_for_requirement` keeps an
  exact gate for the pre-download check (legacy majors like Java 8 still trigger
  the precise download instead of reusing a newer runtime).
- `launch_preview_runtime.rs:745-826` — `select_or_download_java`: exact-installed
  wins; else `resolve_downloadable_java_package` (`:828-873`) walks 16→17;
  substitution logged + surfaced via `emit_java_substitution_notice` (informational
  launcher notice, non-blocking).
- Test updated (M10): `java_runtime.rs` `requirement_falls_back_to_nearest_higher_when_exact_missing`
  replaces `explicit_java_requirement_does_not_select_newer_runtime`;
  added `requirement_prefers_exact_over_higher_and_picks_lowest_qualifying`.

### E — NeoForge 1.20.1 maven coordinates (`fix(neoforge)`, commit `ba9ef3d`)
Root cause: 1.20.1 is the 47.x fork era — publishes under `net.neoforged:forge`
with an MC-prefixed version (`1.20.1-47.x`), not `net.neoforged:neoforge`.
Installer URL used the neoforge coordinate → HTTP 404 → `launch_failed`.
Verified live: forge coord `200`, neoforge coord `404`.
- `launch_preview_runtime.rs:174-207` — `forge_wrapper_installer_artifact`
  special-cases `minecraft_version == "1.20.1"` →
  `net/neoforged/forge/1.20.1-{v}/forge-1.20.1-{v}-installer.jar`.
- TDD: `forge_wrapper_installer_artifact_neoforge_1_20_1_uses_forge_coordinate`.

### A — Legacy natives + virtual asset index (`fix(legacy-natives)`, commit `981cc30`)
Root cause 1 (`UnsatisfiedLinkError: no lwjgl(64)`, 1.12.2 + 1.6.4): vanilla
assigned `mc_data.jvm_arguments` verbatim; legacy versions use the
`minecraftArguments` string → `extract_arguments` yields an **empty** jvm vec →
`-Djava.library.path` never set (modern versions get it from Mojang's
`arguments.jvm`).
- `launch_preview_vanilla.rs:125-126` and `launch_preview.rs:621-622` — route
  vanilla jvm args through `merge_minecraft_and_loader_jvm_arguments` (injects
  natives path when absent, drops redundant `-cp ${classpath}`).

Root cause 2 (1.6.4 `legacy` index, `"virtual":true`): assets are keyed by hash
in the object store but the game reads real filenames.
- `minecraft_downloader.rs:151-156` — `AssetIndexJson.is_virtual`.
- `minecraft_downloader.rs:745-779` — `materialize_virtual_assets` copies objects
  to `assets/virtual/<id>/<path>`.
- `minecraft_downloader.rs:35-37` — `MinecraftVersionData.is_virtual_assets`.
- `launch_preview_models.rs:93-95,202-211` — `${game_assets}` placeholder → virtual
  tree; `launch_preview_runtime.rs:1000-1001` — `${game_assets}` + `${auth_session}`
  substitutions.
- TDD: `legacy_vanilla_jvm_args_gain_natives_path_when_absent`,
  `legacy_virtual_assets_redirect_game_assets_and_session`.
- Validated with the real combo: `vanilla@1.6.4` reaches `running` (in-world),
  not just spawn.

### B — Legacy Forge LaunchWrapper (`fix(legacy-forge)`, commit `04d6d00`)
Root cause: `forge@1.12.2` → LaunchWrapper used `VanillaTweaker` →
`ClassNotFoundException: net.minecraft.client.Minecraft`. Legacy Forge (main
class `net.minecraft.launchwrapper.Launch`) carries its FML tweaker in Prism's
`+tweakers`, which the converter ignored → no `--tweakClass` → VanillaTweaker
fallback.
- `loader_metadata.rs:470-471` — `PrismPackageVersionDetail.tweakers` (`+tweakers`).
- `loader_metadata.rs:687-702` — `loader_metadata_from_prism` appends
  `--tweakClass <t>` per tweaker.
- `launch_preview_runtime.rs:34-36,145-150` — `forge_wrapper_installer_artifact`
  returns `None` unless main class is ForgeWrapper Main, so legacy LaunchWrapper
  Forge skips ForgeWrapper prep. Modern Forge/NeoForge unaffected (verified live:
  they use ForgeWrapper Main + `minecraftArguments`, no `+tweakers`).
- TDD: `converts_prism_legacy_forge_detail_emits_tweak_class`.

### F — Mysterious forge exits (triage)
Full logs read in `launchLogDir` per goal rule.
- **`forge@1.20.5`** — NOT a flake, genuine data issue: `no net.minecraftforge
  metadata version found for Minecraft 1.20.5`. Verified live: Forge's
  `promotions_slim.json` has only `1.20.4` and `1.20.6` — **Forge skipped
  1.20.5** (MC 1.20.4 → 1.20.6 for Forge). Reclassified to negative fixture
  (deviation below). `vanilla/fabric/neoforge@1.20.5` remain positive and pass.
- **`forge@1.20.1` / `forge@1.21.11`** — **environment flakes**, retried once
  per flake policy and both passed. First run: MC started, correct Java selected,
  ForgeWrapper installer prepared, resources + texture atlases rendered, then
  exit 1 with **no Java exception, no crash-report, no hs_err**, at variable
  times (43.8 s / 32.7 s). Logs showed `[EARLYDISPLAY] ERROR DISPLAY / Failed to
  initialize graphics window` + "driver issue" on this NVIDIA/Hyprland/XWayland
  native-display host. Identical code path to `forge@{1.20.4,1.21.1,1.21.4,
  1.21.5,1.21.8}` which always passed ⇒ env/GL flake, not a launcher bug.
  Retry (reds-validate) and full sweep: both `running` at 46 s.

---

## Flaky combos declared (flake policy: one retry)

| combo | first run | retry | classification |
|---|---|---|---|
| forge@1.20.1 | exit 1 @ 43.8 s (EARLYDISPLAY GL) | running @ 46 s | env flake (NVIDIA/XWayland GL) |
| forge@1.21.11 | exit 1 @ 32.7 s (EARLYDISPLAY GL) | running @ 46 s | env flake (NVIDIA/XWayland GL) |

Both passed on the reds-validate retry **and** again on the full sweep — counted
green only after a passing run, not silently.

---

## Deviations from matrix rule 1 (documented)

1. **`forge@1.20.5` positive → negative fixture** (`cc01bdb`). Forge never
   released for 1.20.5 (MC went 1.20.4 → 1.20.6 for Forge). Same nature as the
   authorized `neoforge` pre-1.20.1 reclassification: the loader does not exist
   for that MC version, so the only correct verdict is `launch_failed`. Evidence:
   `promotions_slim.json` (only 1.20.4 / 1.20.6), Prism `net.minecraftforge`
   index has no 1.20.5. This is the **only** deviation beyond the two explicitly
   authorized reclassifications (`neoforge` pre-1.20.1 → negative; `1.6.4` →
   positive).

No successAfter/timeout was lowered, no combo `expect` was weakened, and the
harness verdict logic was untouched.

---

## Display note

xvfb / `xvfb-run` is not installed on this host (contained mode unavailable), so
the sweep ran on **native display** (`DISPLAY=:1`, XWayland) with
`WEBKIT_DISABLE_DMABUF_RENDERER=1` for the launcher webview on NVIDIA. The user
was signalled before the ~1 h occupancy and chose reds-first-then-full.
