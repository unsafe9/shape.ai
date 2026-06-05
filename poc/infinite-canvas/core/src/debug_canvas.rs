use crate::model::{CameraState, SceneSnapshot, WorldRect};
use crate::stats::{CoreFrameStats, CoreHitResult};
use crate::{renderer_backend, serde_wasm};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

#[wasm_bindgen]
pub struct ShapeCanvasCore {
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    canvas: Option<HtmlCanvasElement>,
    context: Option<CanvasRenderingContext2d>,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
}

#[wasm_bindgen]
impl ShapeCanvasCore {
    #[wasm_bindgen(constructor)]
    pub fn new() -> ShapeCanvasCore {
        ShapeCanvasCore {
            scene: None,
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            canvas: None,
            context: None,
            width: 1.0,
            height: 1.0,
            device_pixel_ratio: 1.0,
        }
    }

    pub fn mount(&mut self, canvas: HtmlCanvasElement) -> Result<(), JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("2D canvas context is unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        self.canvas = Some(canvas);
        self.context = Some(context);
        Ok(())
    }

    pub fn resize(&mut self, width: f64, height: f64, device_pixel_ratio: f64) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        self.device_pixel_ratio = device_pixel_ratio.max(1.0);
        if let Some(canvas) = &self.canvas {
            canvas.set_width((self.width * self.device_pixel_ratio).round() as u32);
            canvas.set_height((self.height * self.device_pixel_ratio).round() as u32);
        }
    }

    #[wasm_bindgen(js_name = loadScene)]
    pub fn load_scene(&mut self, scene_json: &str) -> Result<(), JsValue> {
        let scene = serde_json::from_str::<SceneSnapshot>(scene_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid scene snapshot: {error}")))?;
        self.camera = CameraState {
            x: scene.camera.x,
            y: scene.camera.y,
            zoom: scene.camera.zoom,
        };
        self.scene = Some(scene);
        Ok(())
    }

    #[wasm_bindgen(js_name = setCamera)]
    pub fn set_camera(&mut self, x: f64, y: f64, zoom: f64) {
        self.camera = CameraState { x, y, zoom };
    }

    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&self) -> Result<JsValue, JsValue> {
        self.render_debug_canvas()?;
        let Some(scene) = &self.scene else {
            return serde_wasm(CoreFrameStats {
                total_groups: 0,
                total_cards: 0,
                total_edges: 0,
                hit_testable_cards: 0,
                backend: renderer_backend(),
            });
        };
        serde_wasm(CoreFrameStats {
            total_groups: scene.groups.len(),
            total_cards: scene.cards.len(),
            total_edges: scene.edges.len(),
            hit_testable_cards: scene.cards.len(),
            backend: renderer_backend(),
        })
    }

    #[wasm_bindgen(js_name = hitTest)]
    pub fn hit_test(&self, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let Some(scene) = &self.scene else {
            return Ok(JsValue::NULL);
        };
        let world_x = (screen_x - self.camera.x) / self.camera.zoom;
        let world_y = (screen_y - self.camera.y) / self.camera.zoom;
        let hit = scene
            .cards
            .iter()
            .rev()
            .find(|card| {
                world_x >= card.bounds.x
                    && world_x <= card.bounds.x + card.bounds.width
                    && world_y >= card.bounds.y
                    && world_y <= card.bounds.y + card.bounds.height
            })
            .map(|card| CoreHitResult {
                id: card.id.clone(),
                kind: "card".to_string(),
                group_id: Some(card.group_id.clone()),
                field: None,
                port: None,
                world_x,
                world_y,
            });
        match hit {
            Some(value) => serde_wasm(value),
            None => Ok(JsValue::NULL),
        }
    }
}

impl ShapeCanvasCore {
    fn render_debug_canvas(&self) -> Result<(), JsValue> {
        let Some(context) = &self.context else {
            return Ok(());
        };
        context.set_transform(
            self.device_pixel_ratio,
            0.0,
            0.0,
            self.device_pixel_ratio,
            0.0,
            0.0,
        )?;
        context.set_fill_style_str("#f8fafc");
        context.fill_rect(0.0, 0.0, self.width, self.height);
        context.set_transform(
            self.camera.zoom * self.device_pixel_ratio,
            0.0,
            0.0,
            self.camera.zoom * self.device_pixel_ratio,
            self.camera.x * self.device_pixel_ratio,
            self.camera.y * self.device_pixel_ratio,
        )?;
        let Some(scene) = &self.scene else {
            return Ok(());
        };
        context.set_line_width(2.0 / self.camera.zoom.max(0.01));
        for edge in &scene.edges {
            let Some(source) = scene.cards.iter().find(|card| card.id == edge.source) else {
                continue;
            };
            let Some(target) = scene.cards.iter().find(|card| card.id == edge.target) else {
                continue;
            };
            let sx = source.bounds.x + source.bounds.width;
            let sy = source.bounds.y + source.bounds.height / 2.0;
            let tx = target.bounds.x;
            let ty = target.bounds.y + target.bounds.height / 2.0;
            let curve = ((tx - sx).abs() * 0.34).max(80.0);
            context.begin_path();
            context.move_to(sx, sy);
            context.bezier_curve_to(sx + curve, sy, tx - curve, ty, tx, ty);
            context.set_stroke_style_str("rgba(54, 76, 92, 0.45)");
            context.stroke();
        }
        for card in &scene.cards {
            context.begin_path();
            rounded_rect(context, &card.bounds, 18.0)?;
            context.set_fill_style_str("#ffffff");
            context.fill();
            context.set_stroke_style_str("#2f7ee6");
            context.stroke();
            context.set_fill_style_str("#172026");
            context.set_font("700 18px Inter, system-ui, sans-serif");
            context.fill_text(&card.title, card.bounds.x + 16.0, card.bounds.y + 44.0)?;
            context.set_fill_style_str("#66727f");
            context.set_font("500 13px Inter, system-ui, sans-serif");
            context.fill_text(&card.summary, card.bounds.x + 16.0, card.bounds.y + 72.0)?;
        }
        Ok(())
    }
}

fn rounded_rect(
    context: &CanvasRenderingContext2d,
    rect: &WorldRect,
    radius: f64,
) -> Result<(), JsValue> {
    context.move_to(rect.x + radius, rect.y);
    context.arc_to(
        rect.x + rect.width,
        rect.y,
        rect.x + rect.width,
        rect.y + rect.height,
        radius,
    )?;
    context.arc_to(
        rect.x + rect.width,
        rect.y + rect.height,
        rect.x,
        rect.y + rect.height,
        radius,
    )?;
    context.arc_to(rect.x, rect.y + rect.height, rect.x, rect.y, radius)?;
    context.arc_to(rect.x, rect.y, rect.x + rect.width, rect.y, radius)?;
    context.close_path();
    Ok(())
}
