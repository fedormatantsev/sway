//! The masonry half of the editor: a widget tree and the events that reach it.
//!
//! Deliberately depends on none of `wgpu`, `vello`, `imaging_vello`, `bevy` --
//! see the crate manifest. `winit` appears only because `ui-events-winit`
//! takes `&winit::event::WindowEvent`; nothing here draws with it.

pub mod external;

use std::sync::Arc;
use std::time::Instant;

use masonry_core::app::{RenderRoot, RenderRootOptions, RenderRootSignal, VisualLayerPlan, WindowSizePolicy};
use masonry_core::core::{NewWidget, TextEvent, Widget, WindowEvent as MasonryWindowEvent};
use masonry::layout::AsUnit;
use masonry::properties::{Background, Dimensions};
use masonry::widgets::{Flex, Label, SizedBox};
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use winit::dpi::PhysicalSize;

use crate::external::ViewportPlaceholder;

/// The Bevy viewport's fixed footprint in the placeholder layout, in logical
/// pixels. Matches the size `EditorPresenter`'s Task 4 hardcoded rect used
/// (`EDITOR_VIEWPORT_RECT`), purely for visual continuity across Task 5 --
/// nothing requires this exact number now that masonry's widget tree decides
/// the rect. Task 6's `GraphCanvas` will replace this whole placeholder tree.
const VIEWPORT_WIDTH: f64 = 640.0;
const VIEWPORT_HEIGHT: f64 = 360.0;

/// A placeholder root widget that paints something obvious: a full-window
/// panel (via `Dimensions::MAX`, the same property `RenderRoot`'s own
/// internal `LayerStack` uses to always measure the full window) with a
/// visible background, a text label, and -- Task 5 -- a
/// [`ViewportPlaceholder`] child marked `PaintLayerMode::External`. That
/// child's layout box is what `sway_editor::external::viewport_rect` reads
/// back out of the `VisualLayerPlan`; the presenter no longer hardcodes it.
///
/// Task 6 replaces this with `GraphCanvas`.
fn placeholder_root() -> NewWidget<dyn Widget> {
    let label = Label::new("sway editor").prepare();
    let viewport = ViewportPlaceholder::new()
        .prepare()
        .with_props(Dimensions::fixed(VIEWPORT_WIDTH.px(), VIEWPORT_HEIGHT.px()));

    SizedBox::new(
        Flex::column()
            .with_fixed(label)
            .with_fixed(viewport)
            .prepare(),
    )
    .prepare()
    .with_props((
        Dimensions::MAX,
        Background::Color(masonry::theme::ZYNC_900),
    ))
    .erased()
}

/// The masonry widget tree, driven by winit events, one `RenderRoot` per
/// window. There is exactly one window in this app, so exactly one
/// `EditorUi`.
pub struct EditorUi {
    root: RenderRoot,
    reducer: WindowEventReducer,
    scale_factor: f64,
    /// When `redraw` last pumped an anim frame. See `redraw`'s docs: this
    /// host drives masonry's animation clock itself rather than through a
    /// real windowing event, because nothing else in this shell does.
    last_anim_tick: Instant,
}

impl EditorUi {
    pub fn new(size: PhysicalSize<u32>, scale_factor: f64) -> Self {
        let root = RenderRoot::new(
            placeholder_root(),
            // R2 (controller dispatch ruling): the signal sink is a no-op.
            // Masonry emits `RenderRootSignal`s for cursor changes, IME, and
            // window requests (resize, title, exit, ...); a spike driving one
            // hardcoded window with no interactive widgets needs none of
            // them. Dropped silently and deliberately -- Task 8 records this
            // as a known simplification, not a bug to fix here.
            |_signal: RenderRootSignal| {},
            RenderRootOptions {
                default_properties: Arc::new(masonry::theme::default_property_set()),
                use_system_fonts: true,
                size_policy: WindowSizePolicy::User,
                size,
                scale_factor,
                test_font: None,
            },
        );
        Self {
            root,
            reducer: WindowEventReducer::default(),
            scale_factor,
            last_anim_tick: Instant::now(),
        }
    }

    /// Feeds one winit event through `ui-events-winit`'s reducer and, if it
    /// translated to something masonry understands, into the `RenderRoot`.
    ///
    /// Not every winit event translates to a masonry event (e.g. most of
    /// `WindowEvent`'s variants -- `Resized`, `RedrawRequested`,
    /// `CloseRequested`, ... -- reduce to `None`); those are the host's job
    /// (`resize`, `redraw`, the shell's own `CloseRequested` handling), not
    /// this method's.
    pub fn handle_winit_event(&mut self, scale_factor: f64, event: &winit::event::WindowEvent) {
        if let Some(translated) = self.reducer.reduce(scale_factor, event) {
            match translated {
                WindowEventTranslation::Keyboard(k) => {
                    self.root.handle_text_event(TextEvent::Keyboard(k));
                }
                WindowEventTranslation::Pointer(p) => {
                    self.root.handle_pointer_event(p);
                }
            }
        }
    }

    /// Tells the `RenderRoot` about a window resize (and, if it changed, a
    /// scale-factor change). Masonry's own `masonry_winit` host sends
    /// `WindowEvent::Rescale` only when the scale factor actually changes
    /// (winit's `ScaleFactorChanged`), so this mirrors that rather than
    /// unconditionally rescaling every frame.
    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        if (scale_factor - self.scale_factor).abs() > f64::EPSILON {
            self.scale_factor = scale_factor;
            self.root
                .handle_window_event(MasonryWindowEvent::Rescale(scale_factor));
        }
        self.root
            .handle_window_event(MasonryWindowEvent::Resize(size));
    }

    /// Runs masonry's paint pass and returns the resulting visual-layer plan.
    ///
    /// Ignores the `Option<TreeUpdate>` `RenderRoot::redraw` also returns
    /// (R4, controller dispatch ruling): accessibility is out of scope for
    /// M1b.
    ///
    /// Pumps a `WindowEvent::AnimFrame` first -- a real gap found while
    /// implementing Task 5, not a pre-existing part of this API. Masonry
    /// resets a widget's `PaintLayerMode` to `Inline` at the top of every
    /// paint pass and only restores it if that widget's own `paint` method
    /// actually runs, which only happens when something (an event, an anim
    /// tick, ...) has marked it dirty. `ViewportPlaceholder`
    /// (`external.rs`) keeps itself dirty via `request_anim_frame`, but that
    /// request is only serviced if the host actually delivers
    /// `WindowEvent::AnimFrame` -- masonry does not invent a clock on its
    /// own. This host has no other source of frame ticks (the signal sink
    /// that would normally carry `RequestAnimFrame` is a no-op, see `new`'s
    /// docs), so `redraw` supplies one directly, every call, using wall-clock
    /// elapsed time since the last call. Confirmed empirically before this
    /// was wired in: an `External` layer that never receives an anim frame
    /// vanishes from the very next `VisualLayerPlan`.
    pub fn redraw(&mut self) -> VisualLayerPlan {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_anim_tick);
        self.last_anim_tick = now;
        self.root
            .handle_window_event(MasonryWindowEvent::AnimFrame(elapsed));

        self.root.redraw().0
    }

    /// Replays every scene layer into one window-space scene.
    ///
    /// `replay_into` skips `External` layers by construction, which is
    /// exactly wanted: the viewport's pixels come from Bevy, not from
    /// masonry, and the hole they leave in this scene is what the compositor
    /// fills. `imaging::record::Scene` implements `PaintSink` directly (see
    /// `imaging-0.0.1`'s `record.rs:772`), so it can be the sink with no
    /// `Painter` wrapper needed.
    pub fn flatten(plan: &VisualLayerPlan) -> imaging::record::Scene {
        let mut scene = imaging::record::Scene::new();
        plan.replay_into(&mut scene);
        scene
    }
}
