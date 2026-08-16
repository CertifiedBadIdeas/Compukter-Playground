# UART Input Focus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make UART commands line-oriented and keep the Playground keyboard focused on UART input without repeated pointer clicks.

**Architecture:** Keep byte construction in the existing UI-independent `UartInputState`. Give the egui input a stable widget ID and request focus whenever a VM snapshot exists, including the frame after either submission path; pointer-driven controls remain usable because focus affects keyboard routing only.

**Tech Stack:** Rust, egui/eframe, Cargo unit and integration tests.

---

### Task 1: Line-oriented UART submissions

**Files:**
- Modify: `src/ui.rs`
- Test: `src/ui.rs`

- [ ] **Step 1: Change the existing unit test to require a newline**

Update the assertion to:

```rust
assert_eq!(input.take_submission(), Some(b"Echo me!\n".to_vec()));
```

Retain the assertions proving that the visible text is cleared and a second,
empty submission returns `None`.

- [ ] **Step 2: Run the focused test and observe the expected failure**

Run: `cargo test ui::tests::uart_input_remains_visible_until_non_empty_submission -- --exact`

Expected: FAIL because the current value is `Echo me!` without the trailing byte
`0x0a`.

- [ ] **Step 3: Append exactly one newline during submission**

Implement `take_submission` by taking the current string, appending `\n`, and
returning its bytes. Preserve the existing empty-input behavior:

```rust
let mut line = std::mem::take(&mut self.text);
line.push('\n');
Some(line.into_bytes())
```

- [ ] **Step 4: Run the focused test and observe it pass**

Run: `cargo test ui::tests::uart_input_remains_visible_until_non_empty_submission -- --exact`

Expected: PASS.

### Task 2: Persistent keyboard focus

**Files:**
- Modify: `src/ui.rs`

- [ ] **Step 1: Give the UART input a stable ID**

Create the ID near the widget using `ui.make_persistent_id("uart-input")` and
attach it through `TextEdit::id(...)`.

- [ ] **Step 2: Request UART keyboard focus while a VM exists**

After creating the `TextEdit` response, call `response.request_focus()` whenever
`snapshot.is_some()`. Keep the existing Enter detection based on the response,
so Enter and Send both call `submit_uart` and therefore share line construction.

- [ ] **Step 3: Verify the complete Playground**

Run:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit successfully; the ignored NuttX source-build test
remains ignored in the ordinary suite.

- [ ] **Step 4: Commit the implementation**

```bash
git add src/ui.rs docs/superpowers/plans/2026-08-16-uart-input-focus.md
git commit -m "feat(ui): capture UART keyboard input"
```
