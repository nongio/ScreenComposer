use layers::{
    engine::{LayersEngine, NodeRef},
    prelude::taffy,
    types::Point,
};

use crate::workspace::utils::view_render_elements;
use crate::workspace::WindowViewSurface;

#[derive(Clone)]
pub struct DndView {
    engine: layers::prelude::LayersEngine,
    pub view_content: layers::prelude::View<Vec<WindowViewSurface>>,

    pub layer: layers::prelude::Layer,
    pub content_layer: layers::prelude::Layer,
    parent_layer_noderef: NodeRef,
    pub initial_position: Point,
}

impl DndView {
    pub fn new(layers_engine: LayersEngine, parent_layer_noderef: NodeRef) -> Self {
        let layer = layers_engine.new_layer();
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer.set_opacity(0.0, None);
        let content_layer = layers_engine.new_layer();
        content_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        layers_engine.scene_add_layer_to(layer.clone(), Some(parent_layer_noderef));
        layers_engine.scene_add_layer_to(content_layer.clone(), layer.id());

        let render_elements = Vec::new();

        let view_content = layers::prelude::View::new(
            content_layer.clone(),
            render_elements,
            Box::new(view_render_elements),
        );

        Self {
            view_content,
            engine: layers_engine,
            layer,
            content_layer,
            parent_layer_noderef,
            initial_position: Point::default(),
        }
    }
    pub fn set_initial_position(&mut self, point: Point) {
        self.initial_position = point;
    }
}