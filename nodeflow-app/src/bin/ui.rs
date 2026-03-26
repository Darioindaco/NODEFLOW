//! NodeFlow – visuele node-graph editor
//!
//! Vensterindeling:
//! ┌──────────────┬────────────────────────────────────┐
//! │              │                                    │
//! │   Sidebar    │         Canvas (node-graph)        │
//! │  (node-      │                                    │
//! │   picker)    │   [Node]──────────▶[Node]          │
//! │              │                                    │
//! │  + Solid     │                                    │
//! │  + Merge     │                                    │
//! │  + Grade     │                                    │
//! └──────────────┴────────────────────────────────────┘
//!
//! Bediening:
//!   • Rechtermuisklik op canvas  → node toevoegen
//!   • Linkermuisklik + slepen    → node verplaatsen
//!   • Klik op output-poort (▶)  → start verbinding, klik op input-poort om te verbinden
//!   • Rechtermuisklik op node    → node verwijderen
//!   • Middelste muis / Space+sleep → canvas pannen
//!   • Scrollwiel                 → zoom

use eframe::egui::{
    self, Color32, FontId, Painter, Pos2, Rect, Rounding, Stroke,
    Vec2, pos2, vec2,
};
use std::collections::HashMap;

// ── Constanten ────────────────────────────────────────────────────────────────

const NODE_WIDTH: f32     = 170.0;
const HEADER_H: f32       = 28.0;
const ROW_H: f32          = 22.0;
const PORT_R: f32         = 6.0;
const PORT_MARGIN: f32    = 14.0;  // afstand van rand naar port-middelpunt

const COL_HEADER:  Color32 = Color32::from_rgb(55, 55, 70);
const COL_BODY:    Color32 = Color32::from_rgb(42, 42, 52);
const COL_BORDER:  Color32 = Color32::from_rgb(90, 90, 110);
const COL_PORT_IN: Color32 = Color32::from_rgb(100, 200, 120);
const COL_PORT_OUT:Color32 = Color32::from_rgb(220, 160,  60);
const COL_WIRE:    Color32 = Color32::from_rgb(200, 200, 80);
const COL_WIRE_PENDING: Color32 = Color32::from_rgb(255, 255, 120);
const COL_SEL_BORDER: Color32 = Color32::from_rgb(120, 180, 255);

// ── Data-types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UiNodeId(usize);

#[derive(Debug, Clone, PartialEq)]
enum NodeKind {
    SolidColor,
    Merge,
    Grade,
}

impl NodeKind {
    fn label(&self) -> &'static str {
        match self { Self::SolidColor => "Solid Color", Self::Merge => "Merge", Self::Grade => "Grade" }
    }
    fn inputs(&self) -> &'static [&'static str] {
        match self {
            Self::SolidColor => &[],
            Self::Merge      => &["Bg", "Fg"],
            Self::Grade      => &["Input"],
        }
    }
    fn outputs(&self) -> &'static [&'static str] {
        &["Output"]
    }
    fn header_color(&self) -> Color32 {
        match self {
            Self::SolidColor => Color32::from_rgb(80, 60, 110),
            Self::Merge      => Color32::from_rgb(60, 90, 110),
            Self::Grade      => Color32::from_rgb(60, 110, 80),
        }
    }
}

#[derive(Debug, Clone)]
struct UiNode {
    id:   UiNodeId,
    kind: NodeKind,
    /// Positie in canvas-ruimte (niet schermruimte).
    pos:  Pos2,
}

impl UiNode {
    /// Hoogte van het volledige node-rechthoek in canvas-ruimte.
    fn height(&self) -> f32 {
        let rows = self.kind.inputs().len().max(self.kind.outputs().len()).max(1);
        HEADER_H + rows as f32 * ROW_H + 8.0
    }

    /// Rect in canvas-ruimte.
    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, vec2(NODE_WIDTH, self.height()))
    }

    /// Positie van input-poort `i` in canvas-ruimte.
    fn input_port_pos(&self, i: usize) -> Pos2 {
        pos2(
            self.pos.x,
            self.pos.y + HEADER_H + i as f32 * ROW_H + ROW_H * 0.5,
        )
    }

    /// Positie van output-poort `i` in canvas-ruimte.
    fn output_port_pos(&self, i: usize) -> Pos2 {
        pos2(
            self.pos.x + NODE_WIDTH,
            self.pos.y + HEADER_H + i as f32 * ROW_H + ROW_H * 0.5,
        )
    }
}

/// Een voltooide verbinding: van output-poort naar input-poort.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wire {
    from_node: UiNodeId,
    from_port: usize,
    to_node:   UiNodeId,
    to_port:   usize,
}

/// State van een verbinding die nog getrokken wordt.
#[derive(Debug, Clone)]
struct PendingWire {
    from_node: UiNodeId,
    from_port: usize,
    /// Huidige muispositie in canvas-ruimte.
    tip: Pos2,
}

// ── Hoofd-app ─────────────────────────────────────────────────────────────────

struct NodeFlowApp {
    nodes:      HashMap<UiNodeId, UiNode>,
    wires:      Vec<Wire>,
    next_id:    usize,

    // Viewport
    pan:  Vec2,   // canvas-offset in schermruimte
    zoom: f32,

    // Interactie-state
    dragging_node:   Option<(UiNodeId, Vec2)>,  // (id, offset van muis tot node.pos)
    pending_wire:    Option<PendingWire>,
    selected_node:   Option<UiNodeId>,

    // Context-menu
    context_menu_pos: Option<Pos2>,  // schermruimte
}

impl NodeFlowApp {
    fn new() -> Self {
        let mut app = Self {
            nodes:           HashMap::new(),
            wires:           Vec::new(),
            next_id:         1,
            pan:             vec2(200.0, 100.0),
            zoom:            1.0,
            dragging_node:   None,
            pending_wire:    None,
            selected_node:   None,
            context_menu_pos: None,
        };

        // Startgraph: SolidColor → Merge ← SolidColor → Grade
        let a = app.add_node(NodeKind::SolidColor, pos2(80.0,  80.0));
        let b = app.add_node(NodeKind::SolidColor, pos2(80.0, 200.0));
        let m = app.add_node(NodeKind::Merge,      pos2(320.0, 130.0));
        let g = app.add_node(NodeKind::Grade,      pos2(560.0, 130.0));

        app.wires.push(Wire { from_node: a, from_port: 0, to_node: m, to_port: 0 });
        app.wires.push(Wire { from_node: b, from_port: 0, to_node: m, to_port: 1 });
        app.wires.push(Wire { from_node: m, from_port: 0, to_node: g, to_port: 0 });

        app
    }

    fn add_node(&mut self, kind: NodeKind, pos: Pos2) -> UiNodeId {
        let id = UiNodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(id, UiNode { id, kind, pos });
        id
    }

    // ── Coördinaat-omrekeningen ───────────────────────────────────────────────

    fn to_screen(&self, canvas: Pos2) -> Pos2 {
        pos2(
            canvas.x * self.zoom + self.pan.x,
            canvas.y * self.zoom + self.pan.y,
        )
    }

    fn to_canvas(&self, screen: Pos2) -> Pos2 {
        pos2(
            (screen.x - self.pan.x) / self.zoom,
            (screen.y - self.pan.y) / self.zoom,
        )
    }

    fn scale(&self, v: f32) -> f32 { v * self.zoom }

    // ── Treffertest ──────────────────────────────────────────────────────────

    /// Geeft de node en poort terug die het dichtstbij `canvas_pos` ligt,
    /// als die binnen `PORT_R * 2` is.
    fn hit_output_port(&self, canvas_pos: Pos2) -> Option<(UiNodeId, usize)> {
        for node in self.nodes.values() {
            for i in 0..node.kind.outputs().len() {
                let p = node.output_port_pos(i);
                if (p - canvas_pos).length() < PORT_R * 2.0 {
                    return Some((node.id, i));
                }
            }
        }
        None
    }

    fn hit_input_port(&self, canvas_pos: Pos2) -> Option<(UiNodeId, usize)> {
        for node in self.nodes.values() {
            for i in 0..node.kind.inputs().len() {
                let p = node.input_port_pos(i);
                if (p - canvas_pos).length() < PORT_R * 2.0 {
                    return Some((node.id, i));
                }
            }
        }
        None
    }

    fn hit_node(&self, canvas_pos: Pos2) -> Option<UiNodeId> {
        // Loop in omgekeerde volgorde zodat bovenste node eerst treft.
        for node in self.nodes.values() {
            if node.rect().contains(canvas_pos) {
                return Some(node.id);
            }
        }
        None
    }

    // ── Tekenen ───────────────────────────────────────────────────────────────

    fn draw_wire(
        &self,
        painter: &Painter,
        from_canvas: Pos2,
        to_canvas:   Pos2,
        color:       Color32,
        width:       f32,
    ) {
        let from = self.to_screen(from_canvas);
        let to   = self.to_screen(to_canvas);

        // Bezierkromme: horizontale tangenten.
        let dx = ((to.x - from.x).abs() * 0.5).max(60.0 * self.zoom);
        let cp1 = pos2(from.x + dx, from.y);
        let cp2 = pos2(to.x - dx,   to.y);

        let stroke = Stroke::new(width * self.zoom, color);
        // Benader de bezier met 24 lijnstukjes.
        let pts: Vec<Pos2> = (0..=24)
            .map(|i| {
                let t  = i as f32 / 24.0;
                let it = 1.0 - t;
                // Cubische Bézier: B(t) = (1-t)³P0 + 3(1-t)²tP1 + 3(1-t)t²P2 + t³P3
                pos2(
                    it*it*it*from.x + 3.0*it*it*t*cp1.x + 3.0*it*t*t*cp2.x + t*t*t*to.x,
                    it*it*it*from.y + 3.0*it*it*t*cp1.y + 3.0*it*t*t*cp2.y + t*t*t*to.y,
                )
            })
            .collect();

        for w in pts.windows(2) {
            painter.line_segment([w[0], w[1]], stroke);
        }
    }

    fn draw_node(&self, painter: &Painter, node: &UiNode, selected: bool) {
        let tl    = self.to_screen(node.pos);
        let size  = vec2(self.scale(NODE_WIDTH), self.scale(node.height()));
        let rect  = Rect::from_min_size(tl, size);
        let round = Rounding::same(self.scale(5.0));

        // Achtergrond
        painter.rect_filled(rect, round, COL_BODY);

        // Header
        let header_rect = Rect::from_min_size(tl, vec2(size.x, self.scale(HEADER_H)));
        painter.rect_filled(header_rect, Rounding { nw: round.nw, ne: round.ne, sw: 0.0, se: 0.0 }, node.kind.header_color());

        // Rand (geselecteerd = blauw)
        let border_color = if selected { COL_SEL_BORDER } else { COL_BORDER };
        painter.rect_stroke(rect, round, Stroke::new(self.scale(1.5), border_color));

        // Titel
        let font = FontId::proportional(self.scale(12.0));
        painter.text(
            header_rect.center(),
            egui::Align2::CENTER_CENTER,
            node.kind.label(),
            font.clone(),
            Color32::WHITE,
        );

        // Input-poorten
        for (i, &name) in node.kind.inputs().iter().enumerate() {
            let cp = self.to_screen(node.input_port_pos(i));
            painter.circle_filled(cp, self.scale(PORT_R), COL_PORT_IN);
            painter.circle_stroke(cp, self.scale(PORT_R), Stroke::new(1.0, Color32::WHITE));
            painter.text(
                pos2(cp.x + self.scale(PORT_R + 4.0), cp.y),
                egui::Align2::LEFT_CENTER,
                name,
                FontId::proportional(self.scale(10.0)),
                Color32::LIGHT_GRAY,
            );
        }

        // Output-poorten
        for (i, &name) in node.kind.outputs().iter().enumerate() {
            let cp = self.to_screen(node.output_port_pos(i));
            painter.circle_filled(cp, self.scale(PORT_R), COL_PORT_OUT);
            painter.circle_stroke(cp, self.scale(PORT_R), Stroke::new(1.0, Color32::WHITE));
            painter.text(
                pos2(cp.x - self.scale(PORT_R + 4.0), cp.y),
                egui::Align2::RIGHT_CENTER,
                name,
                FontId::proportional(self.scale(10.0)),
                Color32::LIGHT_GRAY,
            );
        }
    }
}

// ── eframe::App impl ──────────────────────────────────────────────────────────

impl eframe::App for NodeFlowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Sidebar ──────────────────────────────────────────────────────────
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(140.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Nodes").strong());
                ui.separator();
                ui.add_space(4.0);

                for kind in [NodeKind::SolidColor, NodeKind::Merge, NodeKind::Grade] {
                    if ui.button(format!("＋  {}", kind.label())).clicked() {
                        // Voeg toe in het midden van het zichtbare canvas.
                        let center = self.to_canvas(ctx.screen_rect().center());
                        self.add_node(kind, center - vec2(NODE_WIDTH * 0.5, 50.0));
                    }
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Tips").small().color(Color32::GRAY));
                ui.label(egui::RichText::new(
                    "• Sleep node om te verplaatsen\n\
                     • Klik output-poort →\n  sleep naar input-poort\n\
                     • Rechts klik node\n  om te verwijderen\n\
                     • Scrollwiel = zoom\n\
                     • Middelste muis = pan"
                ).small().color(Color32::GRAY));
            });

        // ── Canvas ───────────────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(28, 28, 35)))
            .show(ctx, |ui| {
                let (resp, painter) = ui.allocate_painter(
                    ui.available_size(),
                    egui::Sense::click_and_drag(),
                );
                let rect = resp.rect;

                // ── Rasterachtergrond ─────────────────────────────────────
                {
                    let grid  = (self.zoom * 40.0).max(10.0);
                    let ox    = self.pan.x % grid;
                    let oy    = self.pan.y % grid;
                    let color = Color32::from_rgb(38, 38, 48);
                    let mut x = rect.min.x + ox;
                    while x < rect.max.x { painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(0.5, color)); x += grid; }
                    let mut y = rect.min.y + oy;
                    while y < rect.max.y { painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(0.5, color)); y += grid; }
                }

                // ── Input verwerken ───────────────────────────────────────

                // Zoom met scrollwiel
                if resp.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let mpos  = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                        let old_z = self.zoom;
                        self.zoom = (self.zoom * (1.0 + scroll * 0.001)).clamp(0.2, 4.0);
                        // Zoom rondom muispositie.
                        let factor = self.zoom / old_z;
                        self.pan = mpos.to_vec2() + (self.pan - mpos.to_vec2()) * factor;
                    }
                }

                // Pan met middelste muis
                if resp.dragged_by(egui::PointerButton::Middle) {
                    self.pan += resp.drag_delta();
                }

                // Linker muisknop gedrukt
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(mpos) = ctx.input(|i| i.pointer.press_origin()) {
                        let cpos = self.to_canvas(mpos);

                        // Raak output-poort → start verbinding
                        if let Some((nid, pid)) = self.hit_output_port(cpos) {
                            self.pending_wire = Some(PendingWire { from_node: nid, from_port: pid, tip: cpos });
                        }
                        // Raak node → start slepen
                        else if let Some(nid) = self.hit_node(cpos) {
                            let node_pos = self.nodes[&nid].pos;
                            self.dragging_node  = Some((nid, node_pos - cpos));
                            self.selected_node  = Some(nid);
                            self.context_menu_pos = None;
                        } else {
                            self.selected_node    = None;
                            self.context_menu_pos = None;
                        }
                    }
                }

                // Muisbeweging
                if let Some(mpos) = ctx.input(|i| i.pointer.hover_pos()) {
                    let cpos = self.to_canvas(mpos);

                    if let Some((nid, offset)) = self.dragging_node {
                        if let Some(node) = self.nodes.get_mut(&nid) {
                            node.pos = cpos + offset;
                        }
                    }
                    if let Some(pw) = &mut self.pending_wire {
                        pw.tip = cpos;
                    }
                }

                // Loslaten
                if resp.drag_stopped() {
                    // Verbinding voltooien
                    if let Some(pw) = self.pending_wire.take() {
                        if let Some((to_node, to_port)) = self.hit_input_port(pw.tip) {
                            // Verwijder bestaande verbinding op dezelfde input-poort
                            self.wires.retain(|w| !(w.to_node == to_node && w.to_port == to_port));
                            self.wires.push(Wire {
                                from_node: pw.from_node,
                                from_port: pw.from_port,
                                to_node,
                                to_port,
                            });
                        }
                    }
                    self.dragging_node = None;
                }

                // Rechtermuisklik
                if resp.secondary_clicked() {
                    if let Some(mpos) = ctx.input(|i| i.pointer.interact_pos()) {
                        let cpos = self.to_canvas(mpos);
                        if let Some(nid) = self.hit_node(cpos) {
                            // Node verwijderen
                            self.nodes.remove(&nid);
                            self.wires.retain(|w| w.from_node != nid && w.to_node != nid);
                            self.selected_node = None;
                        } else {
                            self.context_menu_pos = Some(mpos);
                        }
                    }
                }

                // ── Tekenen: voltooide verbindingen ───────────────────────
                let wires = self.wires.clone();
                for wire in &wires {
                    if let (Some(fn_), Some(tn)) = (self.nodes.get(&wire.from_node), self.nodes.get(&wire.to_node)) {
                        let from = fn_.output_port_pos(wire.from_port);
                        let to   = tn.input_port_pos(wire.to_port);
                        self.draw_wire(&painter, from, to, COL_WIRE, 2.0);
                    }
                }

                // ── Tekenen: lopende verbinding ───────────────────────────
                if let Some(pw) = &self.pending_wire.clone() {
                    if let Some(fn_) = self.nodes.get(&pw.from_node) {
                        let from = fn_.output_port_pos(pw.from_port);
                        self.draw_wire(&painter, from, pw.tip, COL_WIRE_PENDING, 2.0);
                    }
                }

                // ── Tekenen: nodes ────────────────────────────────────────
                let node_ids: Vec<UiNodeId> = self.nodes.keys().copied().collect();
                for nid in &node_ids {
                    let selected = self.selected_node == Some(*nid);
                    let node = self.nodes[nid].clone();
                    self.draw_node(&painter, &node, selected);
                }

                // ── Context-menu: node toevoegen ──────────────────────────
                if let Some(mpos) = self.context_menu_pos {
                    let cpos = self.to_canvas(mpos);
                    let mut close = false;
                    egui::Window::new("node_add")
                        .title_bar(false)
                        .resizable(false)
                        .fixed_pos(mpos)
                        .show(ctx, |ui| {
                            ui.label(egui::RichText::new("Node toevoegen").strong());
                            ui.separator();
                            for kind in [NodeKind::SolidColor, NodeKind::Merge, NodeKind::Grade] {
                                if ui.button(kind.label()).clicked() {
                                    self.add_node(kind, cpos);
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("NodeFlow Compositor"),
        ..Default::default()
    };
    eframe::run_native(
        "NodeFlow",
        options,
        Box::new(|_cc| Ok(Box::new(NodeFlowApp::new()))),
    )
}
