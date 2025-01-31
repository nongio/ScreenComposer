use lay_rs::{
    engine::LayersEngine,
    prelude::taffy,
    types::Point,
    view::{RenderLayerTree, View},
};

use crate::workspaces::utils::view_render_elements;
use crate::workspaces::WindowViewSurface;

#[derive(Clone)]
pub struct DndView {
    _engine: lay_rs::prelude::LayersEngine,
    pub view_content: lay_rs::prelude::View<Vec<WindowViewSurface>>,

    pub layer: lay_rs::prelude::Layer,
    pub content_layer: lay_rs::prelude::Layer,
    // _parent_layer_noderef: NodeRef,
    pub initial_position: Point,
}

impl DndView {
    pub fn new(layers_engine: LayersEngine) -> Self {
        let layer = layers_engine.new_layer();
        layer.set_key("dnd_view");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // layer.set_opacity(0.0, None);
        let content_layer = layers_engine.new_layer();
        content_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        layers_engine.add_layer(layer.clone());
        layers_engine.append_layer_to(content_layer.clone(), layer.id());

        let render_elements = Vec::new();

        let view_content = View::new("dnd", render_elements, view_render_elements);
        view_content.mount_layer(content_layer.clone());

        Self {
            view_content,
            _engine: layers_engine,
            layer,
            content_layer,
            initial_position: Point::default(),
        }
    }
    pub fn set_initial_position(&mut self, point: Point) {
        self.initial_position = point;
    }
}
