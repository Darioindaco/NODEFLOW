//! NodeFlow – visuele node-graph editor met viewer
//!
//! Vensterindeling:
//! ┌────────────┬──────────────────────────────┬─────────────────┐
//! │  Sidebar   │       Canvas (node-graph)    │    Viewer       │
//! │            │                              │                 │
//! │ + nodes    │  [FileRead]──▶[Grade]──▶...  │  [beeld]        │
//! │ Properties │                              │                 │
//! └────────────┴──────────────────────────────┴─────────────────┘
//!
//! Bediening:
//!   • Rechtermuisklik canvas  → node toevoegen
//!   • Linkermuisklik + sleep  → node verplaatsen
//!   • Klik output-poort (▶)  → sleep naar input-poort om te verbinden
//!   • Rechtermuisklik node   → verwijderen
//!   • Middelste muis / sleep → canvas pannen
//!   • Scrollwiel             → zoom
//!   • ▶ Renderen knop        → bouw DAG en toon beeld in viewer

#[path = "../nodes.rs"]
mod nodes;

use eframe::egui::{
    self, Color32, FontId, Painter, Pos2, Rect, Rounding, Stroke,
    Vec2, pos2, vec2,
};
use nodeflow_buffer::BlendMode;
use nodeflow_core::{CompositorDag, NodeId, Scheduler, SchedulerConfig};
use nodes::{SolidColorNode, MergeNode, GradeNode, FileReadNode};
use std::collections::HashMap;
use std::sync::Arc;

// ── Constanten ────────────────────────────────────────────────────────────────

const NODE_WIDTH: f32  = 170.0;
const HEADER_H: f32    = 28.0;
const ROW_H: f32       = 22.0;
const PORT_R: f32      = 6.0;

const COL_BODY:         Color32 = Color32::from_rgb(42, 42, 52);
const COL_BORDER:       Color32 = Color32::from_rgb(90, 90, 110);
const COL_PORT_IN:      Color32 = Color32::from_rgb(100, 200, 120);
const COL_PORT_OUT:     Color32 = Color32::from_rgb(220, 160,  60);
const COL_WIRE:         Color32 = Color32::from_rgb(200, 200, 80);
const COL_WIRE_PENDING: Color32 = Color32::from_rgb(255, 255, 120);
const COL_SEL_BORDER:   Color32 = Color32::from_rgb(120, 180, 255);

// ── Data-types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UiNodeId(usize);

#[derive(Debug, Clone, PartialEq)]
enum NodeKind {
    SolidColor,
    Merge,
    Grade,
    FileRead,
}

impl NodeKind {
    fn label(&self) -> &'static str {
        match self {
            Self::SolidColor => "Solid Color",
            Self::Merge      => "Merge",
            Self::Grade      => "Grade",
            Self::FileRead   => "File Read",
        }
    }
    fn inputs(&self) -> &'static [&'static str] {
        match self {
            Self::SolidColor | Self::FileRead => &[],
            Self::Merge  => &["Bg", "Fg"],
            Self::Grade  => &["Input"],
        }
    }
    fn outputs(&self) -> &'static [&'static str] { &["Output"] }
    fn header_color(&self) -> Color32 {
        match self {
            Self::SolidColor => Color32::from_rgb(80,  60, 110),
            Self::Merge      => Color32::from_rgb(60,  90, 110),
            Self::Grade      => Color32::from_rgb(60, 110,  80),
            Self::FileRead   => Color32::from_rgb(130, 80,  40),
        }
    }
}

/// Parameters die per node-type kunnen worden aangepast.
#[derive(Debug, Clone)]
enum NodeParams {
    SolidColor { color: [f32; 4] },
    Merge      { blend: usize },   // index in BLEND_MODES
    Grade      { lift: f32, gain: f32, gamma: f32 },
    FileRead   { path: String },
}

impl NodeParams {
    fn default_for(kind: &NodeKind) -> Self {
        match kind {
            NodeKind::SolidColor => Self::SolidColor { color: [0.5, 0.5, 0.5, 1.0] },
            NodeKind::Merge      => Self::Merge { blend: 0 },
            NodeKind::Grade      => Self::Grade { lift: 0.0, gain: 1.0, gamma: 1.0 },
            NodeKind::FileRead   => Self::FileRead { path: String::new() },
        }
    }
}

const BLEND_MODES: &[(&str, BlendMode)] = &[
    ("Over",     BlendMode::Over),
    ("Multiply", BlendMode::Multiply),
    ("Add",      BlendMode::Add),
    ("In",       BlendMode::In),
    ("Out",      BlendMode::Out),
    ("Atop",     BlendMode::Atop),
];

#[derive(Debug, Clone)]
struct UiNode {
    id:     UiNodeId,
    kind:   NodeKind,
    pos:    Pos2,
    params: NodeParams,
}

impl UiNode {
    fn height(&self) -> f32 {
        let rows = self.kind.inputs().len().max(self.kind.outputs().len()).max(1);
        HEADER_H + rows as f32 * ROW_H + 8.0
    }
    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, vec2(NODE_WIDTH, self.height()))
    }
    fn input_port_pos(&self, i: usize) -> Pos2 {
        pos2(self.pos.x, self.pos.y + HEADER_H + i as f32 * ROW_H + ROW_H * 0.5)
    }
    fn output_port_pos(&self, i: usize) -> Pos2 {
        pos2(self.pos.x + NODE_WIDTH, self.pos.y + HEADER_H + i as f32 * ROW_H + ROW_H * 0.5)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Wire {
    from_node: UiNodeId,
    from_port: usize,
    to_node:   UiNodeId,
    to_port:   usize,
}

#[derive(Debug, Clone)]
struct PendingWire {
    from_node: UiNodeId,
    from_port: usize,
    tip:       Pos2,
}

// ── Hoofd-app ─────────────────────────────────────────────────────────────────

struct NodeFlowApp {
    nodes:   HashMap<UiNodeId, UiNode>,
    wires:   Vec<Wire>,
    next_id: usize,

    pan:  Vec2,
    zoom: f32,

    dragging_node:    Option<(UiNodeId, Vec2)>,
    pending_wire:     Option<PendingWire>,
    selected_node:    Option<UiNodeId>,
    context_menu_pos: Option<Pos2>,

    // Viewer
    render_result: Option<(egui::TextureHandle, u32, u32)>,
    render_error:  Option<String>,
}

impl NodeFlowApp {
    fn new() -> Self {
        let mut app = Self {
            nodes: HashMap::new(), wires: Vec::new(), next_id: 1,
            pan: vec2(220.0, 80.0), zoom: 1.0,
            dragging_node: None, pending_wire: None,
            selected_node: None, context_menu_pos: None,
            render_result: None, render_error: None,
        };

        let a = app.add_node(NodeKind::SolidColor, pos2(60.0,  60.0));
        let b = app.add_node(NodeKind::SolidColor, pos2(60.0, 180.0));
        let m = app.add_node(NodeKind::Merge,      pos2(300.0, 110.0));
        let g = app.add_node(NodeKind::Grade,      pos2(540.0, 110.0));

        // Geef de kleuren iets meer contrast voor het demo-beeld
        if let Some(n) = app.nodes.get_mut(&a) {
            n.params = NodeParams::SolidColor { color: [0.8, 0.1, 0.1, 0.6] };
        }
        if let Some(n) = app.nodes.get_mut(&b) {
            n.params = NodeParams::SolidColor { color: [0.1, 0.2, 0.9, 0.6] };
        }

        app.wires.push(Wire { from_node: a, from_port: 0, to_node: m, to_port: 0 });
        app.wires.push(Wire { from_node: b, from_port: 0, to_node: m, to_port: 1 });
        app.wires.push(Wire { from_node: m, from_port: 0, to_node: g, to_port: 0 });
        app
    }

    fn add_node(&mut self, kind: NodeKind, pos: Pos2) -> UiNodeId {
        let id     = UiNodeId(self.next_id);
        self.next_id += 1;
        let params = NodeParams::default_for(&kind);
        self.nodes.insert(id, UiNode { id, kind, pos, params });
        id
    }

    // ── Coördinaten ──────────────────────────────────────────────────────────

    fn to_screen(&self, c: Pos2) -> Pos2 { pos2(c.x * self.zoom + self.pan.x, c.y * self.zoom + self.pan.y) }
    fn to_canvas(&self, s: Pos2) -> Pos2 { pos2((s.x - self.pan.x) / self.zoom, (s.y - self.pan.y) / self.zoom) }
    fn scale(&self, v: f32) -> f32 { v * self.zoom }

    // ── Treffertest ───────────────────────────────────────────────────────────

    fn hit_output_port(&self, cp: Pos2) -> Option<(UiNodeId, usize)> {
        for n in self.nodes.values() {
            for i in 0..n.kind.outputs().len() {
                if (n.output_port_pos(i) - cp).length() < PORT_R * 2.0 { return Some((n.id, i)); }
            }
        }
        None
    }
    fn hit_input_port(&self, cp: Pos2) -> Option<(UiNodeId, usize)> {
        for n in self.nodes.values() {
            for i in 0..n.kind.inputs().len() {
                if (n.input_port_pos(i) - cp).length() < PORT_R * 2.0 { return Some((n.id, i)); }
            }
        }
        None
    }
    fn hit_node(&self, cp: Pos2) -> Option<UiNodeId> {
        self.nodes.values().find(|n| n.rect().contains(cp)).map(|n| n.id)
    }

    // ── DAG bouwen vanuit UI-state ────────────────────────────────────────────

    fn build_dag(&self) -> Result<(CompositorDag, HashMap<UiNodeId, NodeId>), String> {
        let mut dag    = CompositorDag::new();
        let mut id_map: HashMap<UiNodeId, NodeId> = HashMap::new();
        let render_size = (800_u32, 450_u32); // standaard canvas-grootte voor de viewer

        for ui_node in self.nodes.values() {
            let nid = NodeId(ui_node.id.0 as u64);
            let name = format!("{} #{}", ui_node.kind.label(), ui_node.id.0);
            let (w, h) = render_size;

            let boxed: Box<dyn nodeflow_core::Node> = match &ui_node.params {
                NodeParams::SolidColor { color } =>
                    Box::new(SolidColorNode::new(nid, &name, *color, w, h)),
                NodeParams::Merge { blend } =>
                    Box::new(MergeNode::new(nid, &name, BLEND_MODES[*blend].1)),
                NodeParams::Grade { lift, gain, gamma } => {
                    let l = [*lift,  *lift,  *lift,  0.0];
                    let g = [*gain,  *gain,  *gain,  1.0];
                    let gm= [*gamma, *gamma, *gamma, 1.0];
                    Box::new(GradeNode::new(nid, &name, l, g, gm))
                },
                NodeParams::FileRead { path } =>
                    Box::new(FileReadNode::new(nid, &name, path)),
            };

            dag.add_node(boxed);
            id_map.insert(ui_node.id, nid);
        }

        for wire in &self.wires {
            let from = *id_map.get(&wire.from_node).ok_or("from node niet gevonden")?;
            let to   = *id_map.get(&wire.to_node).ok_or("to node niet gevonden")?;
            dag.connect(from, wire.from_port, to, wire.to_port)
               .map_err(|e| e.to_string())?;
        }

        Ok((dag, id_map))
    }

    /// Rendert de DAG en slaat het resultaat op als egui-textuur.
    fn do_render(&mut self, ctx: &egui::Context) {
        self.render_error = None;

        match self.build_dag() {
            Err(e) => { self.render_error = Some(e); return; }
            Ok((dag, _)) => {
                let scheduler = Scheduler::new(SchedulerConfig::default());
                match scheduler.render_frame(&dag, 0, 24.0) {
                    Err(e) => { self.render_error = Some(e.to_string()); }
                    Ok(buf) => {
                        let w = buf.width() as usize;
                        let h = buf.height() as usize;
                        // Converteer f32 RGBA [0,1] → Color32 u8
                        let pixels: Vec<Color32> = buf.as_slice()
                            .chunks_exact(4)
                            .map(|c| Color32::from_rgba_unmultiplied(
                                (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                                (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                                (c[2].clamp(0.0, 1.0) * 255.0) as u8,
                                (c[3].clamp(0.0, 1.0) * 255.0) as u8,
                            ))
                            .collect();

                        let image = egui::ColorImage { size: [w, h], pixels };
                        let tex   = ctx.load_texture("viewer", image, egui::TextureOptions::LINEAR);
                        self.render_result = Some((tex, buf.width(), buf.height()));
                    }
                }
            }
        }
    }

    // ── Node tekenen ─────────────────────────────────────────────────────────

    fn draw_node(&self, painter: &Painter, node: &UiNode, selected: bool) {
        let tl    = self.to_screen(node.pos);
        let size  = vec2(self.scale(NODE_WIDTH), self.scale(node.height()));
        let rect  = Rect::from_min_size(tl, size);
        let round = Rounding::same(self.scale(5.0));

        painter.rect_filled(rect, round, COL_BODY);
        let hr = Rect::from_min_size(tl, vec2(size.x, self.scale(HEADER_H)));
        painter.rect_filled(hr, Rounding { nw: round.nw, ne: round.ne, sw: 0.0, se: 0.0 }, node.kind.header_color());
        painter.rect_stroke(rect, round, Stroke::new(self.scale(1.5), if selected { COL_SEL_BORDER } else { COL_BORDER }));
        painter.text(hr.center(), egui::Align2::CENTER_CENTER, node.kind.label(), FontId::proportional(self.scale(12.0)), Color32::WHITE);

        for (i, &name) in node.kind.inputs().iter().enumerate() {
            let cp = self.to_screen(node.input_port_pos(i));
            painter.circle_filled(cp, self.scale(PORT_R), COL_PORT_IN);
            painter.circle_stroke(cp, self.scale(PORT_R), Stroke::new(1.0, Color32::WHITE));
            painter.text(pos2(cp.x + self.scale(PORT_R + 4.0), cp.y), egui::Align2::LEFT_CENTER, name, FontId::proportional(self.scale(10.0)), Color32::LIGHT_GRAY);
        }
        for (i, &name) in node.kind.outputs().iter().enumerate() {
            let cp = self.to_screen(node.output_port_pos(i));
            painter.circle_filled(cp, self.scale(PORT_R), COL_PORT_OUT);
            painter.circle_stroke(cp, self.scale(PORT_R), Stroke::new(1.0, Color32::WHITE));
            painter.text(pos2(cp.x - self.scale(PORT_R + 4.0), cp.y), egui::Align2::RIGHT_CENTER, name, FontId::proportional(self.scale(10.0)), Color32::LIGHT_GRAY);
        }
    }

    fn draw_wire(&self, painter: &Painter, from_c: Pos2, to_c: Pos2, color: Color32, w: f32) {
        let from   = self.to_screen(from_c);
        let to     = self.to_screen(to_c);
        let dx     = ((to.x - from.x).abs() * 0.5).max(60.0 * self.zoom);
        let cp1    = pos2(from.x + dx, from.y);
        let cp2    = pos2(to.x - dx,   to.y);
        let stroke = Stroke::new(w * self.zoom, color);
        let pts: Vec<Pos2> = (0..=24).map(|i| {
            let t = i as f32 / 24.0; let it = 1.0 - t;
            pos2(it*it*it*from.x + 3.0*it*it*t*cp1.x + 3.0*it*t*t*cp2.x + t*t*t*to.x,
                 it*it*it*from.y + 3.0*it*it*t*cp1.y + 3.0*it*t*t*cp2.y + t*t*t*to.y)
        }).collect();
        for w in pts.windows(2) { painter.line_segment([w[0], w[1]], stroke); }
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for NodeFlowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        // ── Viewer (rechts) ───────────────────────────────────────────────────
        egui::SidePanel::right("viewer")
            .min_width(300.0)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Viewer").strong());
                ui.separator();

                if let Some(err) = &self.render_error.clone() {
                    ui.colored_label(Color32::from_rgb(255, 100, 100), format!("Fout: {}", err));
                }

                if let Some((tex, w, h)) = &self.render_result {
                    let avail  = ui.available_size();
                    let scale  = (avail.x / *w as f32).min((avail.y - 10.0) / *h as f32);
                    let disp   = vec2(*w as f32 * scale, *h as f32 * scale);
                    ui.image((tex.id(), disp));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(format!("{}×{} px", w, h)).small().color(Color32::GRAY));
                } else {
                    ui.add_space(20.0);
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Klik ▶ Renderen\nin de sidebar").color(Color32::GRAY));
                    });
                }
            });

        // ── Sidebar (links) ───────────────────────────────────────────────────
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(160.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Nodes").strong());
                ui.separator();
                ui.add_space(4.0);

                for kind in [NodeKind::SolidColor, NodeKind::Merge, NodeKind::Grade, NodeKind::FileRead] {
                    if ui.button(format!("＋  {}", kind.label())).clicked() {
                        let center = self.to_canvas(ctx.screen_rect().center());
                        let nid = self.add_node(kind, center - vec2(NODE_WIDTH * 0.5, 50.0));
                        self.selected_node = Some(nid);
                    }
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // Render-knop
                let btn = egui::Button::new(
                    egui::RichText::new("▶  Renderen").color(Color32::BLACK)
                ).fill(Color32::from_rgb(80, 200, 100));
                if ui.add(btn).clicked() {
                    self.do_render(ctx);
                }

                // ── Properties van geselecteerde node ─────────────────────
                if let Some(sel_id) = self.selected_node {
                    if let Some(node) = self.nodes.get_mut(&sel_id) {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.label(egui::RichText::new(format!("⚙ {}", node.kind.label())).strong());
                        ui.add_space(4.0);

                        match &mut node.params {
                            NodeParams::SolidColor { color } => {
                                ui.label("Kleur (RGBA):");
                                let labels = ["R", "G", "B", "A"];
                                for (i, label) in labels.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(*label);
                                        ui.add(egui::Slider::new(&mut color[i], 0.0..=1.0).show_value(true));
                                    });
                                }
                            }
                            NodeParams::Merge { blend } => {
                                ui.label("Blend mode:");
                                egui::ComboBox::from_id_source("blend_mode")
                                    .selected_text(BLEND_MODES[*blend].0)
                                    .show_ui(ui, |ui| {
                                        for (i, (name, _)) in BLEND_MODES.iter().enumerate() {
                                            ui.selectable_value(blend, i, *name);
                                        }
                                    });
                            }
                            NodeParams::Grade { lift, gain, gamma } => {
                                ui.horizontal(|ui| { ui.label("Lift "); ui.add(egui::Slider::new(lift,  -0.5..=0.5).show_value(true)); });
                                ui.horizontal(|ui| { ui.label("Gain "); ui.add(egui::Slider::new(gain,   0.0..=4.0).show_value(true)); });
                                ui.horizontal(|ui| { ui.label("Gamma"); ui.add(egui::Slider::new(gamma,  0.1..=4.0).show_value(true)); });
                            }
                            NodeParams::FileRead { path } => {
                                ui.label("Bestandspad:");
                                ui.text_edit_singleline(path);
                                ui.label(egui::RichText::new("(PNG of JPEG)").small().color(Color32::GRAY));
                            }
                        }
                    }
                }

                ui.add_space(16.0);
                ui.separator();
                ui.label(egui::RichText::new(
                    "• Sleep node\n• Output→Input\n• Rechts = wis\n• Scroll = zoom\n• Midden = pan"
                ).small().color(Color32::GRAY));
            });

        // ── Canvas ───────────────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(28, 28, 35)))
            .show(ctx, |ui| {
                let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                let rect = resp.rect;

                // Raster
                {
                    let grid = (self.zoom * 40.0).max(10.0);
                    let color = Color32::from_rgb(38, 38, 48);
                    let mut x = rect.min.x + self.pan.x % grid;
                    while x < rect.max.x { painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(0.5, color)); x += grid; }
                    let mut y = rect.min.y + self.pan.y % grid;
                    while y < rect.max.y { painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(0.5, color)); y += grid; }
                }

                // Zoom
                if resp.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let mp   = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                        let old  = self.zoom;
                        self.zoom = (self.zoom * (1.0 + scroll * 0.001)).clamp(0.2, 4.0);
                        let f    = self.zoom / old;
                        self.pan = mp.to_vec2() + (self.pan - mp.to_vec2()) * f;
                    }
                }

                // Pan (middelste muis)
                if resp.dragged_by(egui::PointerButton::Middle) { self.pan += resp.drag_delta(); }

                // Linker klik
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(mp) = ctx.input(|i| i.pointer.press_origin()) {
                        let cp = self.to_canvas(mp);
                        if let Some((nid, pid)) = self.hit_output_port(cp) {
                            self.pending_wire = Some(PendingWire { from_node: nid, from_port: pid, tip: cp });
                        } else if let Some(nid) = self.hit_node(cp) {
                            let npos = self.nodes[&nid].pos;
                            self.dragging_node = Some((nid, npos - cp));
                            self.selected_node = Some(nid);
                            self.context_menu_pos = None;
                        } else {
                            self.selected_node    = None;
                            self.context_menu_pos = None;
                        }
                    }
                }

                // Muisbeweging
                if let Some(mp) = ctx.input(|i| i.pointer.hover_pos()) {
                    let cp = self.to_canvas(mp);
                    if let Some((nid, off)) = self.dragging_node {
                        if let Some(n) = self.nodes.get_mut(&nid) { n.pos = cp + off; }
                    }
                    if let Some(pw) = &mut self.pending_wire { pw.tip = cp; }
                }

                // Loslaten
                if resp.drag_stopped() {
                    if let Some(pw) = self.pending_wire.take() {
                        if let Some((to_node, to_port)) = self.hit_input_port(pw.tip) {
                            self.wires.retain(|w| !(w.to_node == to_node && w.to_port == to_port));
                            self.wires.push(Wire { from_node: pw.from_node, from_port: pw.from_port, to_node, to_port });
                        }
                    }
                    self.dragging_node = None;
                }

                // Rechtermuisklik
                if resp.secondary_clicked() {
                    if let Some(mp) = ctx.input(|i| i.pointer.interact_pos()) {
                        let cp = self.to_canvas(mp);
                        if let Some(nid) = self.hit_node(cp) {
                            self.nodes.remove(&nid);
                            self.wires.retain(|w| w.from_node != nid && w.to_node != nid);
                            if self.selected_node == Some(nid) { self.selected_node = None; }
                        } else {
                            self.context_menu_pos = Some(mp);
                        }
                    }
                }

                // Verbindingen tekenen
                let wires = self.wires.clone();
                for w in &wires {
                    if let (Some(fn_), Some(tn)) = (self.nodes.get(&w.from_node), self.nodes.get(&w.to_node)) {
                        self.draw_wire(&painter, fn_.output_port_pos(w.from_port), tn.input_port_pos(w.to_port), COL_WIRE, 2.0);
                    }
                }

                // Lopende verbinding
                if let Some(pw) = self.pending_wire.clone() {
                    if let Some(fn_) = self.nodes.get(&pw.from_node) {
                        self.draw_wire(&painter, fn_.output_port_pos(pw.from_port), pw.tip, COL_WIRE_PENDING, 2.0);
                    }
                }

                // Nodes tekenen
                let ids: Vec<UiNodeId> = self.nodes.keys().copied().collect();
                for nid in &ids {
                    let node = self.nodes[nid].clone();
                    self.draw_node(&painter, &node, self.selected_node == Some(*nid));
                }

                // Context-menu
                if let Some(mp) = self.context_menu_pos {
                    let cp = self.to_canvas(mp);
                    let mut close = false;
                    egui::Window::new("ctx_add").title_bar(false).resizable(false).fixed_pos(mp).show(ctx, |ui| {
                        ui.label(egui::RichText::new("Node toevoegen").strong());
                        ui.separator();
                        for kind in [NodeKind::SolidColor, NodeKind::Merge, NodeKind::Grade, NodeKind::FileRead] {
                            if ui.button(kind.label()).clicked() {
                                let nid = self.add_node(kind, cp);
                                self.selected_node = Some(nid);
                                close = true;
                            }
                        }
                        if ui.button("Annuleren").clicked() { close = true; }
                    });
                    if close { self.context_menu_pos = None; }
                }
            });
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "NodeFlow Compositor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1400.0, 800.0])
                .with_title("NodeFlow Compositor"),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(NodeFlowApp::new()))),
    )
}
