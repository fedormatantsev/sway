//! The masonry half of the editor: a widget tree and the events that reach it.
//!
//! Deliberately depends on none of `wgpu`, `vello`, `imaging_vello`, `bevy` --
//! see the crate manifest. `winit` appears only because `ui-events-winit`
//! takes `&winit::event::WindowEvent`; nothing here draws with it.

use std::sync::Arc;

use masonry_core::app::{RenderRoot, RenderRootOptions, RenderRootSignal, VisualLayerPlan, WindowSizePolicy};
use masonry_core::core::{NewWidget, TextEvent, Widget, WindowEvent as MasonryWindowEvent};
use masonry::properties::{Background, Dimensions};
use masonry::widgets::{Label, SizedBox};
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use winit::dpi::PhysicalSize;

/// A placeholder root widget that paints something obvious: a full-window
/// panel (via `Dimensions::MAX`, the same property `RenderRoot`'s own
/// internal `LayerStack` uses to always measure the full window) with a
/// visible background, holding a text label.
///
/// Task 6 replaces this with `GraphCanvas`.
fn placeholder_root() -> NewWidget<dyn Widget> {
    SizedBox::new(Label::new("sway editor").prepare())
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
    pub fn redraw(&mut self) -> VisualLayerPlan {
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
