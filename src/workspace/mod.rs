mod app_switcher;
mod background;
mod dnd_view;
mod dock;
pub mod utils;
mod window_selector;
mod window_view;
mod workspace_selector;
use crate::{
    shell::WindowElement,
    utils::{
        acquire_write_lock_with_retry, image_from_path,
        natural_layout::{natural_layout, LayoutRect},
        Observable, Observer,
    },
};
use core::fmt;
use freedesktop_desktop_entry::{default_paths, DesktopEntry, Iter as DesktopEntryIter};
use layers::{
    engine::{LayersEngine, NodeRef},
    prelude::{taffy, Easing, Interpolate, Layer, TimingFunction, Transition}, types::Size,
};
use layers::skia::{self, Contains};
use smithay::{
    desktop::WindowSurface, input::pointer::CursorImageStatus, reexports::wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface, Resource}, utils::IsAlive, wayland::shell::xdg::XdgToplevelSurfaceData
};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, AtomicI32}, Arc, Mutex, RwLock, Weak
    },
};
use workspace_selector::WorkspaceSelectorView;

pub use background::BackgroundView;
pub use window_selector::{WindowSelection, WindowSelectorState, WindowSelectorView};
pub use window_view::{WindowView, WindowViewBaseModel, WindowViewSurface};

pub use app_switcher::AppSwitcherView;
pub use dnd_view::DndView;
pub use dock::DockView;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Window {
    pub wl_surface: Option<WlSurface>,
    pub window_element: Option<WindowElement>,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    is_fullscreen: bool,
    is_maximized: bool,
    pub is_minimized: bool,
    pub app_id: String,
    pub base_layer: Layer,
}
impl Window {
    pub fn new_with_layer(layer: Layer) -> Self {
        Self {
            base_layer: layer,
            wl_surface: None,
            window_element: None,
            title: "".to_string(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            is_fullscreen: false,
            is_maximized: false,
            is_minimized: false,
            app_id: "".to_string(),
        }
    }
}
#[derive(Clone, Default)]
pub struct Application {
    pub identifier: String,
    pub desktop_name: Option<String>,
    pub icon_path: Option<String>,
    pub icon: Option<skia::Image>,
}
impl PartialEq for Window {
    fn eq(&self, other: &Self) -> bool {
        self.wl_surface == other.wl_surface
    }
}
impl Eq for Window {}
impl Hash for Window {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.wl_surface.hash(state);
    }
}
impl Window {
    pub fn id(&self) -> Option<ObjectId> {
        self.wl_surface.as_ref().map(|s| s.id())
    }
}
impl fmt::Debug for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Application")
            .field("identifier", &self.identifier)
            .field("desktop_name", &self.desktop_name)
            .field("icon_path", &self.icon_path)
            .field("icon", &self.icon.is_some())
            .finish()
    }
}

impl Hash for Application {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identifier.hash(state);
        self.icon_path.hash(state);
        self.desktop_name.hash(state);
        self.icon.as_ref().map(|i| i.unique_id().hash(state));
    }
}

impl PartialEq for Application {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}
impl Eq for Application {}

#[derive(Clone)]
pub struct Workspace {
    model: Arc<RwLock<WorkspaceModel>>,
    // views
    pub app_switcher: Arc<AppSwitcherView>,
    pub window_selector_view: Arc<WindowSelectorView>,
    pub background_view: Arc<BackgroundView>,
    pub workspace_selector_view: Arc<WorkspaceSelectorView>,
    pub dock: Arc<DockView>,

    // views
    pub window_views: Arc<RwLock<HashMap<ObjectId, WindowView>>>,
    // scene
    pub layers_engine: LayersEngine,
    pub workspace_layer: Layer,
    pub windows_layer: Layer,
    pub overlay_layer: Layer,

    // gestures
    pub show_all: Arc<AtomicBool>,
    pub show_desktop: Arc<AtomicBool>,
    pub expose_bin: Arc<RwLock<HashMap<ObjectId, LayoutRect>>>,
    pub show_all_gesture: Arc<AtomicI32>,
    pub show_desktop_gesture: Arc<AtomicI32>,
}

#[derive(Debug, Default, Clone)]
pub struct WorkspaceModel {
    pub applications_cache: HashMap<String, Application>,
    pub windows_cache: HashMap<ObjectId, Window>,

    pub app_windows_map: HashMap<String, Vec<ObjectId>>,
    pub zindex_application_list: Vec<String>,
    pub application_list: VecDeque<String>,

    pub windows_list: Vec<ObjectId>,
    pub minimized_windows: Vec<(ObjectId, WindowElement)>,
    pub current_application: usize,
    pub width: i32,
    observers: Vec<Weak<dyn Observer<WorkspaceModel>>>,
}

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let model = self.model.read().unwrap();

        f.debug_struct("WorkspaceModel")
            .field("applications", &model.applications_cache)
            // .field("application_list", &self.application_list)
            // .field("windows", &self.windows)
            // .field("current_application", &self.current_application)
            .finish()
    }
}

impl Application {
    pub fn new(app_id: &str) -> Self {
        Self {
            identifier: app_id.to_string(),
            ..Default::default()
        }
    }
}
impl Workspace {
    pub fn new(
        layers_engine: LayersEngine,
        cursor_status: Arc<Mutex<CursorImageStatus>>,
    ) -> Arc<Self> {
        let workspace_layer = layers_engine.new_layer();
        workspace_layer.set_key("workspace_view");
        workspace_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        workspace_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        workspace_layer.set_pointer_events(false);
        let background_layer = layers_engine.new_layer();
        background_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        background_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        background_layer.set_opacity(0.0, None);
        let windows_layer = layers_engine.new_layer();
        windows_layer.set_key("windows_container");
        windows_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        windows_layer.set_pointer_events(false);
        let overlay_layer = layers_engine.new_layer();
        overlay_layer.set_key("overlay_view");
        overlay_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        overlay_layer.set_pointer_events(false);
        let workspace_id = layers_engine.scene_add_layer(workspace_layer.clone());
        
        let workspace_selector_layer = layers_engine.new_layer();
        workspace_selector_layer.set_key("workspace_selector_layer");
        workspace_selector_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        workspace_selector_layer.set_pointer_events(false);
        layers_engine.scene_add_layer_to(background_layer.clone(), Some(workspace_id));
        layers_engine.scene_add_layer_to(windows_layer.clone(), Some(workspace_id));
        layers_engine.scene_add_layer_to(workspace_selector_layer.clone(), Some(workspace_id));

        let mut model = WorkspaceModel::default();

        let app_switcher = AppSwitcherView::new(layers_engine.clone());
        let app_switcher = Arc::new(app_switcher);

        model.add_listener(app_switcher.clone());

        let dock = DockView::new(layers_engine.clone());
        let dock = Arc::new(dock);

        layers_engine.scene_add_layer(overlay_layer.clone());


        model.add_listener(dock.clone());
        dock.view_layer.set_position((0.0, -20.0), None);

        let background_view = BackgroundView::new(layers_engine.clone(), background_layer.clone());
        if let Some(background_image) = image_from_path("./resources/background.jpg", None) {
            background_view.set_image(background_image);
        }
        let background_view = Arc::new(background_view);

        let window_selector_view =
            WindowSelectorView::new(layers_engine.clone(), cursor_status.clone());
        let window_selector_view = Arc::new(window_selector_view);

        let workspace_selector_view =
            WorkspaceSelectorView::new(layers_engine.clone(), workspace_selector_layer.clone());

        Arc::new(Self {
            model: Arc::new(RwLock::new(model)),
            app_switcher,
            window_selector_view: window_selector_view.clone(),
            background_view,
            workspace_selector_view: Arc::new(workspace_selector_view),
            dock,
            layers_engine,
            windows_layer,
            overlay_layer,
            workspace_layer,
            show_all: Arc::new(AtomicBool::new(false)),
            show_desktop: Arc::new(AtomicBool::new(false)),
            expose_bin: Arc::new(RwLock::new(HashMap::new())),
            show_all_gesture: Arc::new(AtomicI32::new(0)),
            show_desktop_gesture: Arc::new(AtomicI32::new(0)),
            window_views: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    pub fn with_model<T>(&self, f: impl FnOnce(&WorkspaceModel) -> T) -> T {
        let model = self.model.read().unwrap();
        f(&model)
    }
    pub fn with_model_mut<T>(&self, f: impl FnOnce(&mut WorkspaceModel) -> T) -> T {
        let mut model = self.model.write().unwrap();
        f(&mut model)
    }
    pub fn get_show_all(&self) -> bool {
        self.show_all.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn set_size(&self, width: f32, _height: f32) {
        self.with_model_mut(|model| {
            model.width = width as i32;
            let event = model.clone();
            model.notify_observers(&event);
        });
        
    }

    fn set_show_all(&self, show_all: bool) {
        self.show_all
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get_show_desktop(&self) -> bool {
        self.show_desktop.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_show_desktop(&self, show_all: bool) {
        self.show_desktop
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    #[profiling::function]
    pub(crate) fn update_window(&self, id: &ObjectId, model: &WindowViewBaseModel) {
        let mut workspace_model = self.model.write().unwrap();

        if let Some(window) = workspace_model.windows_cache.get(id) {
            let mut window = window.clone();
            window.x = model.x;
            window.y = model.y;
            window.w = model.w;
            window.h = model.h;
            window.title = model.title.clone();
            window.is_fullscreen = model.fullscreen;
            workspace_model.windows_cache.insert(id.clone(), window.clone());
        }
    }

    // updates the workspace model using elemenets from Space
    pub(crate) fn update_with_window_elements(&self, windows: Vec<(WindowElement, layers::prelude::Layer, WindowViewBaseModel)>)
    // where
        // I: Iterator<Item = (WindowElement, layers::prelude::Layer, WindowViewBaseModel)>,
    {
        {
            if let Ok(mut model_mut) = self.model.write() {
                model_mut.zindex_application_list = Vec::new();
                model_mut.windows_list = Vec::new();
                model_mut.app_windows_map = HashMap::new();
                // model_mut.windows_cache.clear();
            } else {
                return;
            }
        }
        let mut windows_peek = windows.iter()
            .filter(|(w, _l, _state)| w.wl_surface().is_some()) // do we need this?
            .peekable();

        #[allow(clippy::while_let_on_iterator)]
        while let Some((w, l, state)) = windows_peek.next() {
            let surface = w.wl_surface().map(|s| (s.as_ref()).clone()).unwrap();
            smithay::wayland::compositor::with_states(&surface, |states| {
                let attributes: std::sync::MutexGuard<
                    '_,
                    smithay::wayland::shell::xdg::XdgToplevelSurfaceRoleAttributes,
                > = states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap();

                if let Some(app_id) = attributes.app_id.as_ref() {
                    let id = w.wl_surface().unwrap().id();
                    let wl_surface = w.wl_surface().map(|s| (s.as_ref()).clone());
                    let mut window = self.get_window_for_surface(&id)
                        .unwrap_or_else(|| Window::new_with_layer(l.clone()));
                    
                    window.app_id = app_id.to_string();
                    window.wl_surface = wl_surface;
                    window.window_element = Some(w.clone());
                    window.x = state.x;
                    window.y = state.y;
                    window.w = state.w;
                    window.h = state.h;
                    window.title = state.title.clone();

                    let app_index = {
                        let mut model = self.model.write().unwrap();
                        // don't allow duplicates in app switcher
                        // TODO use config
                        let app_index = model
                            .zindex_application_list
                            .iter()
                            .position(|id| id == app_id)
                            .unwrap_or_else(|| {
                                model.zindex_application_list.push(app_id.clone());
                                model.zindex_application_list.len() - 1
                            });
                        if !model.application_list.contains(app_id) {
                            model.application_list.push_front(app_id.clone());
                        }

                        let app = model
                            .applications_cache
                            .entry(app_id.to_owned())
                            .or_insert(Application {
                                identifier: app_id.to_string(),
                                ..Default::default()
                            })
                            .clone();

                        let windows_for_app = model.app_windows_map.entry(app_id.clone()).or_default();
                        let window_id = window.id().unwrap();
                        windows_for_app.push(window_id);
                        drop(model);
                        {
                            if app.icon.is_none() {
                                self.load_async_app_info(app_id);
                            }
                        }
                        let mut model = self.model.write().unwrap();

                        model.windows_cache.insert(id.clone(), window.clone());
                        app_index
                    };

                    {
                        let mut model_mut: std::sync::RwLockWriteGuard<'_, WorkspaceModel> = self.model.write().unwrap();
                        model_mut.windows_cache.insert(id.clone(), window);
                        model_mut.windows_list.push(id);

                        if windows_peek.peek().is_none() {
                            model_mut.current_application = app_index;
                        }
                    }
                }
            });
        }
        // keep only app in application_list that are in zindex_application_list
        {
            let mut model = self.model.write().unwrap();
            let app_list = model.zindex_application_list.clone();
            {
                // update app list
                model
                .application_list
                .retain(|app_id| app_list.contains(app_id));
            }
            {
                // update minimized windows
                let windows_list = model.windows_list.clone();
                model.minimized_windows.retain(|(id, _)| windows_list.contains(id));
            }
        }

        let model = self.model.read().unwrap();
        let event = model.clone();

        model.notify_observers(&event);
    }

    fn load_async_app_info(&self, app_id: &str) {
        tracing::info!("load_async_app_info: {}", app_id);
        let app_id = app_id.to_string();
        let model = self.model.clone();
        // let instance = self.clone();
        // let ctx = None;//self.direct_context.clone();
        tokio::spawn(async move {
            let mut desktop_entry: Option<DesktopEntry<'_>> = None;
            let bytes;
            let path;
            let default_paths = default_paths();
            let path_result = DesktopEntryIter::new(default_paths)
                .find(|path| path.to_string_lossy().contains(&app_id));

            if let Some(p) = path_result {
                path = p.clone();
                let bytes_result = std::fs::read_to_string(&p);
                if let Ok(b) = bytes_result {
                    bytes = b.clone();
                    if let Ok(entry) = DesktopEntry::decode(&path, &bytes) {
                        desktop_entry = Some(entry);
                    }
                }
            }
            if let Some(desktop_entry) = desktop_entry {
                if let Some(mut model_mut) = acquire_write_lock_with_retry(&model) {
                    let icon_path = desktop_entry
                        .icon()
                        .map(|icon| icon.to_string())
                        .and_then(|icon_name| xdgkit::icon_finder::find_icon(icon_name, 512, 1))
                        .map(|icon| icon.to_str().unwrap().to_string());
                    let icon = icon_path
                        .as_ref()
                        .and_then(|icon_path| image_from_path(icon_path, None));

                    let mut app = model_mut
                        .applications_cache
                        .get(&app_id)
                        .unwrap_or(&Application {
                            identifier: app_id.to_string(),
                            ..Default::default()
                        })
                        .clone();
                    if app.icon_path != icon_path {
                        app.desktop_name = desktop_entry.name(None).map(|name| name.to_string());
                        app.icon_path = icon_path;
                        app.icon = icon.clone();
                        tracing::info!("loaded: {:?}", app);
                        model_mut.applications_cache.insert(app_id, app);
                        model_mut.notify_observers(&model_mut.clone());
                    }
                }
            }
        });
    }

    pub fn expose_show_all(&self, delta: f32, end_gesture: bool) {
        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let mut show_all = self.get_show_all();
        let mut animation_duration = 0.200;
        if end_gesture {
            if show_all {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    show_all = false;
                } else {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                }
            } else {
                animation_duration = 0.200;
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                } else {
                    new_gesture = 0;
                    show_all = false;
                }
            }

            self.set_show_all(show_all);
        }

        let delta = new_gesture as f32 / 1000.0;
        self.show_all_gesture
            .store(new_gesture, std::sync::atomic::Ordering::Relaxed);

        let workspace_selector_height = 250.0;
        let padding_top = 10.0;
        let padding_bottom = 10.0;

        let size = self.workspace_layer.render_size();
        let screen_size_w = size.x;
        let screen_size_h = size.y - padding_top - padding_bottom - workspace_selector_height;
        let model = self.model.read().unwrap();
        let windows = model
            .windows_list
            .iter()
            .filter_map(|w| {
                let w = self.get_window_for_surface(w).unwrap();
                if w.is_minimized {
                    None
                } else {
                    Some(w.clone())
                }
            })
            .collect();

        let mut bin = self.expose_bin.write().unwrap();
        if bin.is_empty() {
            let layout_rect =
                LayoutRect::new(0.0, workspace_selector_height, screen_size_w, screen_size_h);
            *bin = natural_layout(&windows, &layout_rect, false);
        }

        let mut state = WindowSelectorState {
            rects: vec![],
            current_selection: None,
        };

        let mut delta = delta.max(0.0);
        delta = delta.powf(0.65);

        let mut index = 0;

        let mut transition = Some(Transition::ease_in(animation_duration));
        if !end_gesture {
            // in the middle of the gesture
            transition = None;
        }

        let workspace_selector_y = (-200.0).interpolate(&0.0, delta);
        let workspace_opacity = 0.0.interpolate(&1.0, delta);
        self.workspace_selector_view.layer.set_position(
            layers::types::Point {
                x: 0.0,
                y: workspace_selector_y,
            },
            transition,
        );
        self.workspace_selector_view
            .layer
            .set_opacity(workspace_opacity, transition);
        let dock_y = (-20.0).interpolate(&250.0, delta);
        self.dock.view_layer.set_position((0.0, dock_y), transition);

        let mut changes = Vec::new();

        let animation = transition.map(|t| self.layers_engine.new_animation(t, false));
        for window in model.windows_list.iter() {
            let window = self.get_window_for_surface(window).unwrap();
            if window.is_minimized {
                continue;
            }
            let id = window.wl_surface.as_ref().unwrap().id();
            if let Some(rect) = bin.get(&id) {
                let to_x = rect.x;
                let to_y = rect.y;
                let to_width = rect.width;
                let to_height = rect.height;
                let (window_width, window_height) = (window.w, window.h);

                let scale_x = to_width / window_width;
                let scale_y = to_height / window_height;
                let scale = scale_x.min(scale_y).min(1.0);

                let window_rect = WindowSelection {
                    x: rect.x,
                    y: rect.y,
                    w: (window_width * scale),
                    h: (window_height * scale),
                    visible: true,
                    window_title: window.title.clone(),
                    index,
                };
                index += 1;
                state.rects.push(window_rect);
                let scale = 1.0.interpolate(&scale, delta);
                let delta = delta.clamp(0.0, 1.0);

                let x = window.x.interpolate(&to_x, delta);
                let y = window.y.interpolate(&to_y, delta);


                let translation = window
                    .base_layer
                    .change_position(layers::types::Point { x, y });
                let scale = window
                    .base_layer
                    .change_scale(layers::types::Point { x: scale, y: scale });
                changes.push(translation);
                changes.push(scale);
            }
        }
        self.layers_engine.add_animated_changes(&changes, animation);
        self.window_selector_view.view.update_state(&state);
        animation.map(|a| self.layers_engine.start_animation(a, 0.0));
        if end_gesture {
            *bin = HashMap::new();
        }
    }

    pub fn expose_show_desktop(&self, delta: f32, end_gesture: bool) {
        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_desktop_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let show_desktop = self.get_show_desktop();

        let model = self.model.read().unwrap();

        if end_gesture {
            if show_desktop {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                } else {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                }
            } else {
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                } else {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                }
            }
        } else if !show_desktop {
            new_gesture -= MULTIPLIER as i32;
        }

        let delta = new_gesture as f32 / 1000.0;

        let delta = delta.clamp(0.0, 1.0);

        let mut transition = Some(Transition::ease_in(0.5));

        if !end_gesture {
            // in the middle of the gesture
            transition = None;
        }

        for window in model.windows_list.iter() {
            let window = self.get_window_for_surface(window).unwrap();
            if window.is_minimized {
                continue;
            }
            let to_x = -window.w;
            let to_y = -window.h;
            let x = window.x.interpolate(&to_x, delta);
            let y = window.y.interpolate(&to_y, delta);

            window
                .base_layer
                .set_position(layers::types::Point { x, y }, transition);
        }
    }
    pub fn get_app_windows(&self, app_id: &str) -> Vec<ObjectId> {
        let model = self.model.read().unwrap();
        model
            .app_windows_map
            .get(app_id)
            .cloned()
            .unwrap_or_default()
    }
    pub fn get_current_app(&self) -> Option<Application> {
        let model = self.model.read().unwrap();
        let app_id = model.zindex_application_list[model.current_application].clone();
        model.applications_cache.get(&app_id).cloned()
    }
    pub fn get_current_app_windows(&self) -> Vec<ObjectId> {
        self.get_current_app()
            .map(|app| self.get_app_windows(&app.identifier))
            .unwrap_or_default()
    }
    pub fn quit_app(&self, app_id: &str) {
        for window_id in self.get_app_windows(app_id) {
            let window = self.get_window_for_surface(&window_id).unwrap();
            if let Some(we) = window.window_element.as_ref() {
                match we.underlying_surface() {
                    WindowSurface::Wayland(t) => t.send_close(),
                    #[cfg(feature = "xwayland")]
                    WindowSurface::X11(w) => {
                        let _ = w.close();
                    }
                }
            }
        }
    }

    pub fn quit_current_app(&self) {
        let current_app = self.get_current_app();
        if let Some(app) = current_app {
            self.quit_app(&app.identifier);
        }
    }

    pub fn quit_appswitcher_app(&self) {
        let appswitcher_app = self.app_switcher.get_current_app();

        if let Some(app) = appswitcher_app {
            self.quit_app(&app.identifier);
        }
    }

    pub fn get_window_for_surface(&self, id: &ObjectId) -> Option<Window> {
        let model = self.model.read().unwrap();
        model.windows_cache.get(id).cloned()
    }

    pub fn is_cursor_over_dock(&self, x: f32, y: f32) -> bool {
        self.dock.alive() && 
        self
            .dock
            .view_layer
            .render_bounds_transformed()
            .contains(skia::Point::new(x, y))
    }

    pub fn get_or_add_window_view(
        &self,
        object_id: &ObjectId,
        parent_layer_id: NodeRef,
        window: WindowElement,
    ) -> WindowView {
        let mut window_views = self.window_views.write().unwrap();

        let insert = window_views
            .entry(object_id.clone())
            .or_insert_with(|| WindowView::new(self.layers_engine.clone(), parent_layer_id, window));
        insert.clone()
    }
    pub fn remove_window_view(&self, object_id: &ObjectId) {
        let mut window_views = self.window_views.write().unwrap();
        if let Some(_view) = window_views.remove(object_id) {}
    }
    pub fn get_window_view(&self, id: &ObjectId) -> Option<WindowView> {
        let window_views = self.window_views.read().unwrap();

        window_views.get(id).cloned()
    }
    pub fn set_window_view(&self, id: &ObjectId, window_view: WindowView) {
        let mut window_views = self.window_views.write().unwrap();

        window_views.insert(id.clone(), window_view);
    }

    pub fn minimize_window(&self, id: &ObjectId, we: &WindowElement) {
        let mut model = self.model.write().unwrap();

        if let Some(mut window) = model.windows_cache.get(id).cloned() {
            window.is_minimized = true;
            model.windows_cache.insert(id.clone(), window.clone());
            model.minimized_windows.push((id.clone(), we.clone()));
            
            if let Some(view) = self.get_window_view(id) {
                let (drawer, _) = self.dock.add_window_element(&window);
                    
                view.window_layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    ..Default::default()
                });
                self.layers_engine.scene_add_layer_to_positioned(view.window_layer.clone(), drawer.clone());
                // bounds are calculate after this call
                let drawer_bounds = drawer.render_bounds_transformed();
                view.minimize(skia::Rect::from_xywh(drawer_bounds.x(), drawer_bounds.y(), 130.0, 130.0));
                
                let view_ref = view.clone();
                drawer.on_change_size(move |layer: &Layer, _| {
                    let bounds = layer.render_bounds_transformed();
                    view_ref.genie_effect.set_destination(bounds);
                    view_ref.genie_effect.apply();
                }, false);
            }
            let event = model.clone();
            model.notify_observers(&event);
        }
    }
    pub fn unminimize_window(&self, id: &ObjectId) {
        let mut model = self.model.write().unwrap();

        if let Some(mut window) = model.windows_cache.get(id).cloned() {
            if let Some(view) = self.get_window_view(&id) {
                window.is_minimized = false;
                model.windows_cache.insert(id.clone(), window.clone());
                model.minimized_windows.retain(|(wid, _)| wid != id);

                if let Some(drawer) = self.dock.remove_window_element(&window) {
                    let engine_ref = self.layers_engine.clone();
                    let windows_layer_ref = self.windows_layer.clone();
                    let layer_ref = view.window_layer.clone();
                    self.layers_engine.update(0.0);


                    let drawer_bounds = drawer.render_bounds_transformed();
                    let pos_x = window.x;
                    let pos_y = window.y;
                    drawer.set_size(Size::points(0.0, 130.0), Transition {
                        delay: 0.2,
                        timing: TimingFunction::ease_out_quad(0.3),
                    })
                    .on_start(move |_layer: &Layer, _| {
                        layer_ref.remove_draw_content();
                        engine_ref.scene_add_layer_to_positioned(layer_ref.clone(), windows_layer_ref.clone());
                        layer_ref.set_position((pos_x,pos_y), Transition::ease_out(0.3));
                    }, true)
                    .then(move |layer: &Layer, _| {
                        layer.remove();
                    });
                    

                    view.unminimize(drawer_bounds);
                }
            }

            let event = model.clone();
            model.notify_observers(&event);
        }
    }
}

impl Observable<WorkspaceModel> for WorkspaceModel {
    fn add_listener(&mut self, observer: std::sync::Arc<dyn Observer<WorkspaceModel>>) {
        let observer = std::sync::Arc::downgrade(&observer);
        self.observers.push(observer);
    }

    fn observers<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = std::sync::Weak<dyn Observer<WorkspaceModel>>> + 'a> {
        Box::new(self.observers.iter().cloned())
    }
}
