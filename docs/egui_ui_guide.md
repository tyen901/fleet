## Authoritative egui Architecture Guide (Code-Behind, Shared Behavior for UI + CLI)

This guide defines the **exact shapes** you should implement across:

* `_app`: authoritative state + behavior (single source of truth)
* `_ui`: egui presenter (screens render snapshots, call intents)
* `_cli`: CLI presenter (commands call same intents, show same state)

Non-negotiable rules (summarized): **UI renders state; it does not consume events**; **no UI-facing state channels**; **UI renders snapshots only**; **slow UI must not affect domain execution**.    

---

# 1) Crate responsibilities and dependency direction

## `_app` (authoritative)

Owns:

* all domain state machines and invariants
* background work scheduling/execution
* service interfaces (traits) used by presenters
* read models (“snapshots”) designed for presentation
* domain errors

Exports:

* `trait` ports (`AuthService`, `SyncService`, etc.)
* read model structs/enums (`SyncModel`, etc.)
* error types (`AppError`, etc.)
* optional: an `AppHandle` that contains concrete service implementations (but keep concrete types out of `_ui` via trait objects)

## `_ui` (egui presenter)

Owns:

* screen stack framework
* `Screen` trait + implementations
* `UiContext` capability injection
* navigation and ephemeral UX events (banners)

Must not own:

* reducers, domain state machines, progress logic, “interpretation” of domain meaning

## `_cli` (CLI presenter)

Owns:

* command parsing and output formatting
* polling loops for “watch/status”

Must not own:

* business logic or duplicated state transitions (calls `_app` ports only)

---

# 2) Canonical contracts (exact shapes)

## 2.1 Screen contract (`_ui`)

Implement exactly this interface and rules. 

```rust
pub trait Screen {
    fn id(&self) -> ScreenId;
    fn name(&self) -> &'static str;

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext);

    fn on_push(&mut self, _ctx: &mut UiContext) {}
    fn on_pop(&mut self, _ctx: &mut UiContext) {}
    fn on_pause(&mut self, _ctx: &mut UiContext) {}
    fn on_resume(&mut self, _ctx: &mut UiContext) {}
}
```

**Screen rules (enforced):**

* screens hold **only ephemeral view state** (text buffers, selected tab, scroll state)
* screens **must not cache domain state across frames**
* screens **must not subscribe to events**
* screens **must not mutate global state directly** 

## 2.2 UiContext capability injection (`_ui`)

`UiContext` is the only bridge into application behavior and navigation. It must carry **trait objects only**. 

```rust
pub struct UiContext<'a> {
    pub frame: FrameInfo,

    pub nav: &'a mut dyn Navigation,
    pub screens: &'a dyn Screens,

    pub events: &'a dyn Events,

    pub auth: &'a dyn AuthService,
    pub sync: &'a dyn SyncService,
    pub data: &'a dyn DataService,
}
```

Rules:

* traits only (`&dyn Trait`)
* no concrete implementations
* no generic dispatch enums
* no global “command handler” 

## 2.3 Navigation contract (`_ui`)

Navigation is buffered during rendering and applied after the frame. Navigation ops contain **screens only**. 

```rust
pub trait Navigation {
    fn push(&mut self, screen: Box<dyn Screen>);
    fn pop(&mut self);
    fn replace(&mut self, screen: Box<dyn Screen>);
    fn close_app(&mut self);
}
```

Hard restriction:

* no domain payloads
* no commands
* no closures 

## 2.4 Ephemeral UX events (`_ui`)

Events are allowed only for transient UX (banners), never for state reconstruction or progress. 

```rust
pub enum UiEvent {
    Warning { message: String },
    Error { message: String },
}
```

Rules:

* fire-and-forget
* not replayed, not queryable
* never used for progress/state transport 

---

# 3) `_app` service model (authoritative behavior + shared read models)

## 3.1 Service trait shape (ports)

A service exposes:

* a read model snapshot
* intent methods (commands)

Example from your contract: 

```rust
pub trait SyncService {
    fn snapshot(&self) -> SyncModel;
    fn start(&self);
    fn cancel(&self);
}
```

Rules:

* `snapshot()` is fast and non-blocking
* intents update authoritative state immediately (or enqueue work, but state transition is owned by `_app`)
* no UI-facing event streams 

## 3.2 Read model (“snapshot”) shape

Read models are **presentation-ready** structures used by both UI and CLI.

Example (from contract): 

```rust
pub struct SyncModel {
    pub phase: String,
    pub percent: u8,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_verified: u64,
    pub files_up_to_date: u64,
    pub error: Option<AppError>,
    pub finished: bool,
}
```

**Instruction:** Put “can_*” flags and user-facing status meaning into the read model to avoid duplicating rules across `_ui` and `_cli` (e.g., `can_start`, `can_cancel`, `status_line`, `blocking_error`).

## 3.3 Authoritative internal model vs. read model

Inside `_app`, you may maintain a richer internal `DomainModel`. The snapshot returned to presenters is the **read model** (often a subset + derived fields).

**Do not** expose `Arc<RwLock<DomainModel>>` to `_ui` or `_cli`.

## 3.4 Internal domain events (allowed) vs UI events (forbidden)

Internal “domain events” are allowed **inside `_app` only** as an implementation detail (e.g., worker emits `SyncEvent` and applies it to the model). Your contract shows reducer-style application inside the service. 

UI must never consume these.

---

# 4) Snapshot implementation requirements (how to make it real)

You must implement `snapshot()` so it is:

* cheap (no deep clones per frame for large structures)
* does not block writers
* safe if UI freezes (domain continues) 

## 4.1 Standard pattern (recommended)

**Publish immutable read models** and let readers grab the latest.

Concrete options:

* Small read models: return by value (`SyncModel`) if it is genuinely cheap.
* Medium/large read models: return `Arc<ReadModel>` and make `snapshot()` return `Arc<…>`.

**Rule of thumb:** if it can grow (lists, trees, logs), use `Arc` snapshots.

## 4.2 Prohibited snapshot implementations

* `snapshot()` that deep clones large models every frame
* holding a long-lived `RwLock` read guard through rendering
* any UI subscription to a progress stream (explicitly forbidden) 

---

# 5) `_ui` shell loop (exact responsibilities)

The shell does exactly: build `UiContext`, render top screen, render global HUD, apply nav ops, drain events. 

Shell must not:

* interpret domain meaning
* contain reducers
* forward domain events 

---

# 6) How to implement any page or interaction (FAQ-style)

## 6.1 “How do I create a new page/screen?”

1. Create a struct in `_ui`:

   * fields: only ephemeral view state (text buffers, selection, local UI toggles)
2. Implement `Screen`:

   * in `ui()`, read snapshots from `ctx.*.snapshot()`
   * render from snapshot
   * on interaction, call intent methods or push/pop screens

Template:

```rust
pub struct SettingsScreen {
    // ephemeral
    api_key_buf: String,
}

impl Screen for SettingsScreen {
    fn id(&self) -> ScreenId { ScreenId::Settings }
    fn name(&self) -> &'static str { "Settings" }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &mut UiContext) {
        let auth = ctx.auth.snapshot(); // presentation model

        ui.label(auth.status_line);

        ui.text_edit_singleline(&mut self.api_key_buf);

        if ui.button("Save").clicked() {
            if let Err(e) = ctx.auth.set_api_key(self.api_key_buf.clone()) {
                ctx.events.emit(UiEvent::Error { message: e.to_string() });
            } else {
                ctx.events.emit(UiEvent::Warning { message: "Saved".into() });
            }
        }
    }
}
```

## 6.2 “How do I do navigation from a click?”

* Call `ctx.nav.push(Box::new(NextScreen::new(...)))`
* Do not pass domain payloads through nav ops; if you need identity, pass a lightweight ID used to query `_app` state.

## 6.3 “How do I handle a form (validation, submit, error)?”

* Keep input buffers in the screen (ephemeral)
* On submit:

  * call a `_app` intent that performs validation and returns `Result`
  * show errors as ephemeral UX event, or include validation errors in the read model if they are persistent

Do not build a second validation engine in `_ui`.

## 6.4 “How do I trigger long-running work (sync, import, download)?”

* UI button calls intent: `ctx.sync.start()`
* `_app` spawns/queues background work (internal)
* background work updates authoritative state
* UI renders progress from snapshot each frame (never from a channel) 

## 6.5 “How do I show progress bars / live counters?”

Always:

```rust
let snap = ctx.sync.snapshot();
ui.label(&snap.phase);
ui.add(egui::ProgressBar::new(snap.percent as f32 / 100.0));
```

This is the canonical pattern. 

Never:

```rust
while let Some(ev) = rx.recv().await { ... }
```

Delete on sight. 

## 6.6 “How do I show errors?”

* Persistent domain failures: appear in the snapshot (`snap.error`)
* Transient UI notifications: `ctx.events.emit(UiEvent::Error { ... })`

Do not “replay” errors via an event stream.

## 6.7 “How do I do a modal dialog / confirmation?”

* Keep `show_confirm: bool` in the screen (ephemeral)
* When confirmed: call intent
* When canceled: toggle flag only

Do not represent confirmations as domain state unless the workflow truly spans time.

## 6.8 “How do I implement a list/details page?”

* Snapshot contains list of items (IDs + display fields)
* Screen stores `selected_id: Option<ItemId>` (ephemeral)
* Details panel reads snapshot again (or uses `selected_id` to query another read model)

## 6.9 “How do I implement tabs?”

* `active_tab: enum` in screen
* Each tab renders from snapshots
* Any action calls intents

## 6.10 “How do I make UI refresh when background work progresses?”

* UI already pulls snapshots every frame.
* If you need more frequent repaint, use egui repaint requests from the shell, but **do not** create a UI state stream.

(Implementation detail: the shell may call `egui_ctx.request_repaint()` based on a timer.)

---

# 7) CLI parity (same behavior, different view)

## 7.1 “How does CLI map to the same model?”

* CLI subcommand = call the same `_app` intent methods
* CLI “status” = print the same snapshot fields
* CLI “watch” = poll snapshot periodically and re-render output

CLI never consumes progress events; it behaves like the UI does: render the latest snapshot.

---

# 8) Enforcement checklist (merge gate)

Before merging any `_ui` code:

* UI removal does not affect domain correctness
* UI cannot block domain progress
* state lives outside screens
* no UI-facing queues/channels exist
* all progress is snapshot-based
* events are ephemeral only
* screens are replaceable without breaking logic 

Final rule: if the UI ever needs to “catch up”, the design is wrong.