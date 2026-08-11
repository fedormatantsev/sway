//! Spec "Verify before implementing" item 3: `rfd::AsyncFileDialog`'s future
//! must be pollable from the shell's redraw loop, which has no executor under
//! it. If this ever fails, `shell.rs`'s `Dialog` has to become a thread plus a
//! channel instead.
//!
//! `#[ignore]` because it opens a real file dialog on some platforms and would
//! block CI. Run it by hand once, when adding `rfd` or bumping it:
//! `cargo test -p sway-app --test rfd_pollable -- --ignored`

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[test]
#[ignore = "opens a real file dialog; run by hand when adding or bumping rfd"]
fn an_async_file_dialog_future_polls_pending_without_an_executor() {
    let mut future = pin!(rfd::AsyncFileDialog::new().pick_file());
    let mut cx = Context::from_waker(Waker::noop());

    // One poll, no executor, no runtime. Pending is the pass: the dialog is
    // open and nobody has picked anything yet.
    assert!(matches!(future.as_mut().poll(&mut cx), Poll::Pending));
}
