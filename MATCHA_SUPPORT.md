# Matcha support integration branch

This branch is a long-lived integration branch for bringing Klei's Matcha
Flavoured datapack to Pumpkin 26.2. It is deliberately not an upstream pull
request: generic Pumpkin fixes must be extracted into focused branches and sent
upstream separately.

Current status: **function runtime, persistent scheduling, persistent stopwatches,
and entity type selectors implemented; not yet playable**. Pumpkin can discover
the combined pack, validate its 26.2 format range, load all 95 functions, run
`minecraft:load` and `minecraft:tick`, persist delayed function calls and
real-time stopwatches across restarts, and resolve namespaced direct entity
types plus vanilla and pack-defined entity type tags. Live Matcha testing now
reaches the missing gameplay commands listed below.

## Inputs audited

The baseline below was measured on 2026-08-02 from:

- `Matcha_Flavoured_1_03.zip`: combined datapack and resource pack for 26.2.
- `matcha-vanilla-flavoured-1-03.zip`: client resource-pack-only variant. Its
  `pack.mcmeta` currently describes itself as `1.01 for 26.2` despite the 1.03
  filename.

The archives are not committed or redistributed by this branch.

The combined pack contains:

- 5,196 entries: 2,418 under `data/` and 2,775 under `assets/`.
- 95 functions and 2,095 datapack JSON files.
- `minecraft:load` and `minecraft:tick` function tags.
- 309 top-level `execute` commands, 43 `scoreboard` commands, 20 `function`
  commands, 11 `schedule` commands, and 7 `stopwatch` commands.
- JSON-heavy content including 1,060 recipes, 282 loot tables, 235 villager
  trades, 223 advancements, 75 world-generation entries, 24 enchantments, 68
  trade sets, and custom timeline, dimension, instrument, and jukebox data.

All 2,095 datapack JSON files parse as valid JSON.

## Integrated prerequisites

| Area | Integration commit | Upstream work | Status here |
| --- | --- | --- | --- |
| Scoreboard commands, criteria, persistence, display slots, render types, automatic criteria, and WIT API | `31d1cad4` | [Pumpkin #2658](https://github.com/Pumpkin-MC/Pumpkin/pull/2658) | Integrated on current upstream |
| Execute modifier chaining | `9750ac25` | [Pumpkin #2609](https://github.com/Pumpkin-MC/Pumpkin/pull/2609) | Integrated on current upstream |
| False/empty modifier result propagation | `7427a928` | Follow-up branch `agent/command-chain-pr-ready` | Integrated on current upstream |
| Scoreboard WIT definitions | `588e5c53ef7b6d52f508bd0cd6174dba6259b11c` | [pumpkin-plugin-wit #25](https://github.com/Pumpkin-MC/pumpkin-plugin-wit/pull/25) | Mirrored, merged with current upstream WIT, and pinned |

The submodule URL intentionally points to
[`Vaspyyy/pumpkin-plugin-wit`](https://github.com/Vaspyyy/pumpkin-plugin-wit),
branch `matcha-scoreboard-api`. The gitlink pins the exact commit, so a clone of
this branch does not depend on the contributor fork remaining available.

## Compatibility baseline

| Matcha dependency | Measured use | Pumpkin status on this branch | Next action |
| --- | ---: | --- | --- |
| Pack discovery, `pack.mcmeta`, reload, and resource registry | Whole pack | Folder and ZIP discovery plus the 107.1 format check are implemented. Reload and data-driven registries are still missing. | Add enabled-pack ordering and `/reload` when registry loading begins. |
| Function loading and `minecraft:load` / `minecraft:tick` | 95 functions; both standard tags | Implemented with nested function tags, command-chain limits, recursion limits, and one-time line diagnostics. | Extend the runtime for function macros only when a target pack requires them. |
| Scoreboards | 43 direct commands; 38 score conditions; 7 score result stores | Covered by the integrated scoreboard work. Matcha's objectives were created by its load function and confirmed to persist across a restart. | Fix the two Matcha lines that set `sleepTimerScore` before creating that objective. |
| `/schedule function` | 11 calls | Implemented with function/tag callbacks, append/replace/clear semantics, game-time tracking, duplicate suppression, and `scheduled_events.dat` persistence. | Exercise one of Matcha's scheduled mechanics in-game after its downstream commands work. |
| `/stopwatch` and `execute if/unless stopwatch` | 7 creates; 41 conditions | Implemented with vanilla-style elapsed-millisecond persistence; offline time is excluded. | Exercise a stopwatch-driven mechanic after selector parsing works. |
| `execute if items` | 47 conditions | Missing. | Add item-stack predicate matching and the execute condition. |
| `execute if predicate` | 7 conditions | Missing. Loot predicates are not loaded. | Load predicates and expose them to execute and selectors. |
| `execute if biome` | 1 condition | Missing. | Add biome lookup condition. |
| `execute on vehicle` | 10 modifiers | Missing. | Add the relation modifier after function execution works. |
| `execute store result entity` | 3 stores | Missing. Only score result/success storage is implemented. | Add NBT-backed entity storage. |
| Entity selector types and type tags | 67 selector occurrences | Implemented for namespaced direct types and vanilla/custom/nested entity type tags, including tag replacement. Predicate evaluation is asynchronous so entity tags, teams, scores, advancements, and NBT do not block Tokio workers. | Extend the same data-pack tag registry pattern to item and biome conditions as those features land. |
| Recipes, loot tables, advancements, enchantments, trades, and worldgen | 2,095 JSON files total | Pumpkin currently uses generated/static vanilla data rather than resources from an enabled pack. | Add registries incrementally; recipes and advancements are good early vertical slices. |
| Client assets | 2,775 files in the combined pack; separate resource-only zip available | Pumpkin can advertise a Java resource-pack URL, but it does not serve this local archive automatically. | Host the resource-only zip and configure its URL/SHA-1 when gameplay support is ready. |

This means scoreboard support is necessary, but it is no longer the critical
path. The shortest path to a visible Matcha milestone is:

1. discover and validate the pack (**implemented**);
2. load and run functions plus the load/tick tags (**implemented**);
3. add `/schedule` (**implemented**) and the missing execute conditions used by Matcha;
4. load tags and predicates;
5. make one data-driven content family work end-to-end, then expand registry
   coverage.

## Definition of the first playable milestone

The first milestone is not “all 2,095 JSON files work.” It is a small, testable
vertical slice:

1. Pumpkin discovers the pack in a world's `datapacks/` directory.
2. The pack passes its 26.2 format check.
3. `main:setup/load` runs through `minecraft:load` and sends Matcha's loaded
   message.
4. `main:setup/tick` runs every tick without an unknown-command or dispatcher
   routing error.
5. Matcha's scoreboard setup persists through a restart.
6. One mechanic that uses a scheduled function works end-to-end.

Only after this is green should the branch claim that Matcha is playable.

### 2026-08-02 live runtime result

An isolated Pumpkin server booted directly against the unmodified
`Matcha_Flavoured_1_03.zip` and reported:

- 1 data pack loaded;
- 95 functions loaded;
- 1 `minecraft:load` function resolved;
- `minecraft:tick` ran for 25 seconds without a crash or repeated-log flood;
- a clean shutdown and save;
- a second boot found the Matcha objectives already present, confirming
  scoreboard persistence.

With scheduling, stopwatches, and entity type selectors implemented, the first
remaining failures are the remaining Matcha execute conditions. Empty-player
selector failures and the two `sleepTimerScore` ordering errors are expected
pack/runtime-context issues rather than loader failures.

### 2026-08-03 scheduler integration result

An isolated two-boot test scheduled a function 1,200 ticks in the future,
saved the world, and restarted Pumpkin. `/schedule clear test:later` removed
exactly one event after restart, proving that the event was restored from
`scheduled_events.dat`. Scheduling the same function for two ticks then ran it
and changed its test score from 0 to 1. The file uses the vanilla 26.2 callback
shape and the scheduler uses the persisted `Time` game clock rather than a
process-local tick counter.

### 2026-08-03 stopwatch integration result

The 26.2 `/stopwatch create`, `query`, `restart`, and `remove` operations and
`execute if/unless stopwatch` conditions are implemented. A two-boot live test
restarted a stopwatch, kept the server offline for over a minute, and observed
about three seconds of elapsed runtime immediately after restart, confirming
that persisted elapsed milliseconds exclude offline time. The unmodified
Matcha archive then booted without any standalone stopwatch command or
condition failures; its remaining lines containing `stopwatch` fail earlier at
the namespaced entity-selector parser.

### 2026-08-03 entity type selector integration result

Focused parser and registry tests cover `type=minecraft:armor_stand`,
`type=#main:mundane_hostiles`, nested tags, the generated 26.2 vanilla tags,
and data-pack `replace`. In a live server using the unmodified Matcha archive,
an armor stand matched its namespaced direct type, and a zombie matched both
`#minecraft:undead` and Matcha's `#main:mundane_hostiles` tag. Spawning those
entities also exposed an existing selector panic caused by blocking Tokio
mutexes while evaluating scoreboard tags; selector predicates now await those
locks, and the same live commands and Matcha tick loop remained stable.

## Weekly upstream maintenance

Normal weekly updates use merges because `matcha-support` is a published
integration branch. Rebasing it every week would rewrite the commit IDs that
testers and builds may already use.

```bash
git fetch origin
git switch matcha-support
git merge origin/master
git submodule sync --recursive
git submodule update --init --recursive
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features
cargo test --doc --workspace --all-features
cargo build --release
git push fork matcha-support
```

Before merging, inspect the upstream range and the local patch delta:

```bash
git log --oneline HEAD..origin/master
git diff --stat origin/master...HEAD
git submodule status
```

When upstream merges one of the prerequisite patches, do not keep a duplicate
copy indefinitely. Build a replacement integration tip from current
`origin/master`, carry forward only the still-unmerged Matcha commits, run the
full checks, and update the published branch with `--force-with-lease`. Announce
that exceptional history rewrite before anyone updates a test server.

## Upstream contribution policy

- Do not open an upstream PR from `matcha-support`.
- Start each generic contribution from current `origin/master`.
- Keep scoreboard, dispatcher, datapack loading, commands, and individual data
  registries in separate PRs unless a dependency makes separation impossible.
- Include focused tests and a Matcha-derived reproduction without committing
  the Matcha archives.
- Reconcile the integration branch only after the upstream PR is merged or its
  final patch is stable.

## Clone and build

```bash
git clone --recurse-submodules --branch matcha-support \
  https://github.com/Vaspyyy/Pumpkin.git Pumpkin-matcha-support
cd Pumpkin-matcha-support
cargo build --release
```

At the current stage this builds a server that loads, executes, and schedules
Matcha's functions, but the missing commands, execute conditions, selectors,
and data-driven registries above still prevent the datapack from being
playable.
