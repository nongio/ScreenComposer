use smithay::{
    delegate_input_method_manager, delegate_pointer_constraints,
    desktop::{PopupKind, PopupManager},
    input::pointer::PointerHandle,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Rectangle,
    wayland::{
        input_method::{InputMethodHandler, PopupSurface},
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler},
        seat::WaylandFocus,
    },
};
use tracing::warn;

use super::{Backend, ScreenComposer};

impl<BackendData: Backend> InputMethodHandler for ScreenComposer<BackendData> {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!("Failed to track popup: {}", err);
        }
    }

    fn popup_repositioned(&mut self, _: PopupSurface) {}

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, smithay::utils::Logical> {
        self.workspaces
            .spaces_elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(parent)).then(|| window.geometry())
            })
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(@<BackendData: Backend + 'static> ScreenComposer<BackendData>);

impl<BackendData: Backend> PointerConstraintsHandler for ScreenComposer<BackendData> {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // XXX region
        let Some(current_focus) = pointer.current_focus() else {
            return;
        };
        if current_focus.wl_surface().as_deref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                constraint.unwrap().activate();
            });
        }
    }
}
delegate_pointer_constraints!(@<BackendData: Backend + 'static> ScreenComposer<BackendData>);
