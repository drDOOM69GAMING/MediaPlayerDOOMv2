#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use rand::Rng;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use lofty::prelude::*;

const APP_NAME: &str = "Random Shuffle Player";
const APP_VERSION: &str = "2.4.0";

const AUDIO_FORMATS: [&str; 16] = [
    "mp3", "wav", "flac", "m4a", "m4b", "m4p", "ogg", "oga", "aac", "aiff", "aif", "wma", "wv", "mpc", "opus", "webm",
];

const THEME_NAMES: [&str; 5] = ["Winamp", "Matrix", "Cyberpunk", "Amber", "Ocean"];

const NO_WINDOW: u32 = 0x0800_0000;

fn parse_yt_progress(s: &str) -> Option<(f32, String, String)> {
    let t = s.trim();
    if !t.starts_with("[download]") || !t.contains('%') {
        return None;
    }
    let rest = t.trim_start_matches("[download]").trim();
    let pct: f32 = rest.split('%').next()?.trim().parse().ok()?;
    let speed = rest
        .split(" at ")
        .nth(1)
        .map(|x| x.split(" ETA").next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let eta = rest
        .split(" ETA ")
        .nth(1)
        .map(|x| x.trim().to_string())
        .unwrap_or_default();
    Some((pct, speed, eta))
}

fn is_yt_log_line(s: &str) -> bool {
    s.contains("% of") || s.contains("[download]") || s.contains("[ExtractAudio]")
        || s.starts_with("Destination:") || s.starts_with("Deleting original")
        || s.starts_with("[info]") || s.starts_with("[ffmpeg]") || s.starts_with("ERROR")
}

struct ThemePalette {
    bg: Color32,
    fg: Color32,
    accent: Color32,
    highlight: Color32,
    btn_bg: Color32,
    btn_fg: Color32,
    playing_fg: Color32,
    art_bg: Color32,
}

fn hex(c: &str) -> Color32 {
    let h = c.trim_start_matches('#');
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0);
    Color32::from_rgb(r, g, b)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t).round() as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t).round() as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t).round() as u8,
        a.a(),
    )
}

fn lighten(c: Color32, f: f32) -> Color32 {
    mix(c, Color32::from_rgb(255, 255, 255), f)
}

fn darken(c: Color32, f: f32) -> Color32 {
    mix(c, Color32::from_rgb(0, 0, 0), f)
}

fn retro_btn(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32) -> egui::Response {
    let w = label.chars().count() as f32 * 7.0 + 24.0;
    let h = 20.0;
    retro_btn_ex(ui, label, fg, base, w, h)
}

fn retro_btn_ex(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32, w: f32, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let off = if pressed { 1.0 } else { 0.0 };
        let r = rect.shrink(0.5).translate(egui::vec2(off, off));
        let rr = 2.0;
        let top = mix(base, Color32::from_rgb(255, 255, 255), if pressed { 0.24 } else if hovered { 0.42 } else { 0.32 });
        let bot = darken(base, if pressed { 0.50 } else { 0.22 });
        painter.rect_filled(r, rr, bot);
        let hh = r.height();
        painter.rect_filled(
            egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + hh * 0.42)),
            rr,
            top,
        );
        let fr = r.shrink(2.0);
        painter.rect_filled(fr, 1.5, if pressed { darken(base, 0.08) } else { mix(base, Color32::BLACK, 0.13) });
        painter.line_segment(
            [fr.left_top(), fr.right_top()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.45) }),
        );
        painter.line_segment(
            [fr.left_top(), fr.left_bottom()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.3) }),
        );
        painter.line_segment(
            [fr.right_top(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.72)),
        );
        painter.line_segment(
            [fr.left_bottom(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.78)),
        );
        if hovered && !pressed {
            painter.rect_stroke(fr, 1.5, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
        }
        painter.text(
            fr.center() + egui::vec2(0.0, off),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(if h > 26.0 { 11.5 } else { 10.0 }),
            fg,
        );
    }
    resp
}

fn deck_key(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32) -> egui::Response {
    let w = (label.chars().count() as f32 * 6.4 + 20.0).max(40.0);
    let h = 40.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let pressed = resp.is_pointer_button_down_on();
        let hovered = resp.hovered();
        let off = if pressed { 1.0 } else { 0.0 };
        let r = rect.shrink(1.0).translate(egui::vec2(off, off));
        let top = mix(base, Color32::from_rgb(255, 255, 255), if pressed { 0.24 } else { 0.40 });
        let bot = darken(base, if pressed { 0.50 } else { 0.22 });
        let rr = 2.0;
        painter.rect_filled(r, rr, bot);
        let hh = r.height();
        painter.rect_filled(
            egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + hh * 0.42)),
            rr,
            top,
        );
        let fr = r.shrink(3.0);
        painter.rect_filled(fr, 1.5, if pressed { darken(base, 0.08) } else { mix(base, Color32::BLACK, 0.14) });
        painter.line_segment(
            [fr.left_top(), fr.right_top()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.45) }),
        );
        painter.line_segment(
            [fr.left_top(), fr.left_bottom()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.3) }),
        );
        painter.line_segment(
            [fr.right_top(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.72)),
        );
        painter.line_segment(
            [fr.left_bottom(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.78)),
        );
        if hovered && !pressed {
            painter.rect_stroke(fr, 1.5, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
        }
        painter.text(
            fr.center() + egui::vec2(0.0, off),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(10.5),
            fg,
        );
    }
    resp
}

fn tape_reel_at(painter: &egui::Painter, c: egui::Pos2, r: f32, fg: Color32, base: Color32, ang: f32) {
    painter.circle_filled(c, r, base);
    painter.circle(c, r, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.62)));
    painter.circle(c, r * 0.80, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    let step = std::f32::consts::TAU / 6.0;
    for k in 0..6 {
        let a = ang + k as f32 * step;
        let p1 = c + egui::vec2(a.cos() * r * 0.30, a.sin() * r * 0.30);
        let p2 = c + egui::vec2(a.cos() * r * 0.90, a.sin() * r * 0.90);
        painter.line_segment([p1, p2], egui::Stroke::new(1.0, fg));
    }
    painter.circle(c, r * 0.30, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.45)));
    painter.circle_filled(c, r * 0.16, fg);
}

fn cassette(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    anim: f32,
    side: char,
    songs: u32,
    cap: u32,
    accent: Color32,
    base: Color32,
    playing: bool,
    wind: f32, // 0..1 how hard we're winding
    wind_dir: f32,
) {
    let (slot, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    painter.rect_filled(slot, 4.0, darken(base, 0.55));
    painter.rect_stroke(slot, 4.0, egui::Stroke::new(1.0, darken(base, 0.75)), egui::StrokeKind::Inside);
    painter.rect_filled(slot.shrink(3.0), 3.0, darken(base, 0.42));
    painter.line_segment(
        [slot.left_top() + egui::vec2(3.0, 5.0), slot.right_top() + egui::vec2(-3.0, 5.0)],
        egui::Stroke::new(1.0, lighten(base, 0.1)),
    );
    painter.line_segment(
        [slot.left_bottom() + egui::vec2(3.0, -5.0), slot.right_bottom() + egui::vec2(-3.0, -5.0)],
        egui::Stroke::new(1.0, lighten(base, 0.05)),
    );
    if anim >= 0.97 {
        painter.text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            "EJECT",
            egui::FontId::monospace(11.0),
            darken(accent, 0.2),
        );
        return;
    }
    let t = ui.ctx().input(|i| i.time) as f32;
    let tr = slot.translate(egui::vec2(0.0, anim * (height + 12.0)));
    let shell_top = mix(Color32::from_gray(215), Color32::from_gray(255), 0.30);
    let shell_bot = Color32::from_gray(150);
    let rr = 4.0;
    painter.rect_filled(tr, rr, shell_bot);
    let hh = tr.height();
    painter.rect_filled(
        egui::Rect::from_min_max(tr.min, egui::pos2(tr.max.x, tr.min.y + hh * 0.45)),
        rr,
        shell_top,
    );
    painter.rect_stroke(tr, rr, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
    let label = egui::Rect::from_min_max(
        tr.min + egui::vec2(8.0, 5.0),
        egui::pos2(tr.max.x - 8.0, tr.min.y + hh * 0.42),
    );
    painter.rect_filled(label, 2.0, Color32::from_gray(232));
    painter.rect_stroke(label, 2.0, egui::Stroke::new(1.0, Color32::from_gray(170)), egui::StrokeKind::Inside);
    painter.text(label.left_top() + egui::vec2(6.0, 3.0), egui::Align2::LEFT_TOP, "MPDOOM", egui::FontId::monospace(9.0), Color32::from_rgb(160, 30, 30));
    painter.text(label.right_bottom() + egui::vec2(-6.0, -3.0), egui::Align2::RIGHT_BOTTOM, format!("SIDE {}  {}/{}", side, songs, cap.min(99)), egui::FontId::monospace(9.0), Color32::from_gray(60));
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(tr.min.x + 8.0, tr.min.y + hh * 0.46),
            egui::pos2(tr.max.x - 8.0, tr.max.y - 6.0),
        ),
        3.0,
        Color32::from_gray(34),
    );
    let win = egui::Rect::from_min_max(
        egui::pos2(tr.min.x + 14.0, tr.min.y + hh * 0.50),
        egui::pos2(tr.max.x - 14.0, tr.max.y - 12.0),
    );
    painter.rect_filled(win, 2.0, Color32::from_rgb(20, 24, 28));
    let strand_c = if playing { accent } else { Color32::from_gray(120) };
    let strand_w = win.height() * 0.30;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(win.min.x, win.center().y - strand_w * 0.5),
            egui::pos2(win.max.x, win.center().y + strand_w * 0.5),
        ),
        1.0,
        strand_c.linear_multiply(if playing { 0.85 } else { 0.35 }),
    );
    let reel_r = win.height() * 0.42;
    let lc = egui::pos2(win.min.x + reel_r + 4.0, win.center().y);
    let rc = egui::pos2(win.max.x - reel_r - 4.0, win.center().y);
    let base_speed = if playing { 6.0 } else { 0.0 };
    let l_speed = base_speed + wind * (if wind_dir < 0.0 { 14.0 } else { -3.0 });
    let r_speed = base_speed + wind * (if wind_dir > 0.0 { 14.0 } else { -3.0 });
    tape_reel_at(&painter, lc, reel_r, strand_c, Color32::from_rgb(52, 58, 64), t * l_speed);
    tape_reel_at(&painter, rc, reel_r, strand_c, Color32::from_rgb(52, 58, 64), t * r_speed);
    for s in [
        tr.min + egui::vec2(10.0, 8.0),
        egui::pos2(tr.max.x - 10.0, tr.min.y + 8.0),
        tr.min + egui::vec2(10.0, hh - 9.0),
        egui::pos2(tr.max.x - 10.0, tr.min.y + hh - 9.0),
    ] {
        painter.circle_filled(s, 2.2, Color32::from_gray(120));
        painter.circle_filled(s, 1.2, Color32::from_gray(60));
    }
}

fn rotary(ui: &mut egui::Ui, label: &str, val01: f32, fg: Color32, base: Color32) -> f32 {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 46.0), egui::Sense::drag());
    let mut out = val01;
    if resp.dragged() {
        let dy = ui.input(|i| i.pointer.delta().y);
        out = (val01 - dy / 160.0).clamp(0.0, 1.0);
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let c = rect.center();
        let rad = 15.0;
        painter.circle_filled(c, rad + 2.0, darken(base, 0.75));
        painter.circle_filled(c, rad, mix(base, Color32::from_rgb(255, 255, 255), 0.16));
        painter.circle_filled(c, rad - 2.5, darken(base, 0.15));
        let ang = -2.3561945_f32 + val01 * 4.7123890_f32;
        let p1 = c + egui::vec2(ang.cos() * rad * 0.20, ang.sin() * rad * 0.20);
        let p2 = c + egui::vec2(ang.cos() * (rad - 4.0), ang.sin() * (rad - 4.0));
        painter.line_segment([p1, p2], egui::Stroke::new(3.0, fg));
        for k in 0..5 {
            let a = -2.3561945_f32 + k as f32 * 1.1780972_f32;
            let q = c + egui::vec2(a.cos() * (rad + 2.0), a.sin() * (rad + 2.0));
            let q2 = c + egui::vec2(a.cos() * (rad + 4.0), a.sin() * (rad + 4.0));
            painter.line_segment([q, q2], egui::Stroke::new(1.0, darken(base, 0.6)));
        }
        painter.circle_filled(c, 2.0, fg);
        painter.text(
            egui::pos2(rect.center().x, rect.max.y - 9.5),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(8.5),
            fg,
        );
    }
    out
}

fn plaque(ui: &mut egui::Ui, width: f32, text: &str, fg: Color32, base: Color32) {
    let h = 15.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect, 1.0, darken(base, 0.12));
    painter.rect_stroke(rect, 1.0, egui::Stroke::new(1.0, darken(base, 0.55)), egui::StrokeKind::Inside);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(9.0), fg);
}

fn grill_dots(painter: &egui::Painter, rect: egui::Rect, dot: Color32) {
    let step = 7.0;
    let mut x = rect.min.x + step / 2.0;
    while x < rect.max.x {
        let mut y = rect.min.y + step / 2.0;
        while y < rect.max.y {
            painter.circle_filled(egui::pos2(x, y), 1.4, dot);
            y += step;
        }
        x += step;
    }
}

fn speaker_woofer(ui: &mut egui::Ui, d: f32, base: Color32, accent: Color32, pulse: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let c = rect.center();
    let r = d * 0.5 - 3.0;
    painter.circle_filled(c, r + 2.0, darken(base, 0.6));
    painter.circle(c, r + 2.0, Color32::TRANSPARENT, egui::Stroke::new(1.5, darken(base, 0.75)));
    painter.circle_filled(c, r, darken(base, 0.42));
    painter.circle_filled(c, r * 0.88, darken(base, 0.30));
    painter.circle(c, r * 0.88, Color32::TRANSPARENT, egui::Stroke::new(1.0, lighten(base, 0.08)));
    painter.circle(c, r * 0.80, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    let cone_r = r * 0.80;
    let surge = 1.0 + pulse * 0.28;
    let flex = cone_r * (1.06 + pulse * 0.20);
    painter.circle(c, flex, Color32::TRANSPARENT, egui::Stroke::new(1.4, mix(base, Color32::BLACK, 0.55 - pulse * 0.35)));
    if pulse > 0.02 {
        painter.circle_filled(c, r * 0.95, accent.linear_multiply(0.05 + pulse * 0.12));
    }
    painter.circle_filled(c, cone_r, darken(base, 0.14));
    for i in 1..=6 {
        let k = i as f32 / 6.0;
        let rr = cone_r * k * surge;
        let col = mix(base, Color32::BLACK, if i % 2 == 0 { 0.30 } else { 0.38 });
        painter.circle(c, rr.max(1.0), Color32::TRANSPARENT, egui::Stroke::new(1.0, col));
    }
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
        let p1 = c + egui::vec2(a.cos() * cone_r * 0.28, a.sin() * cone_r * 0.28);
        let p2 = c + egui::vec2(a.cos() * cone_r * 0.94 * surge, a.sin() * cone_r * 0.94 * surge);
        painter.line_segment([p1, p2], egui::Stroke::new(1.0, darken(base, 0.5)));
    }
    painter.circle_filled(c, cone_r * 0.30 * surge, lighten(base, 0.12 + pulse * 0.22));
    painter.circle(c, cone_r * 0.30 * surge, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    let glow = mix(accent, Color32::WHITE, 0.3);
    painter.line_segment(
        [
            c + egui::vec2(-r * 0.55, -r * 0.45),
            c + egui::vec2(-r * 0.15, -r * 0.62),
        ],
        egui::Stroke::new(1.5, glow.linear_multiply(0.25)),
    );
}

fn lcd(ui: &mut egui::Ui, text: &str, led: Color32, base: Color32) {
    let w = text.chars().count() as f32 * 9.2 + 18.0;
    let h = 24.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let inner = rect.shrink(1.0);
    painter.rect_filled(inner, 2.0, darken(base, 0.30));
    painter.line_segment(
        [inner.left_top(), inner.right_top()],
        egui::Stroke::new(1.0, darken(base, 0.72)),
    );
    painter.line_segment(
        [inner.left_top(), inner.left_bottom()],
        egui::Stroke::new(1.0, darken(base, 0.72)),
    );
    painter.line_segment(
        [inner.right_top(), inner.right_bottom()],
        egui::Stroke::new(1.0, lighten(base, 0.25)),
    );
    painter.line_segment(
        [inner.left_bottom(), inner.right_bottom()],
        egui::Stroke::new(1.0, lighten(base, 0.25)),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(15.0),
        led,
    );
}

fn engrave(painter: &egui::Painter, rect: egui::Rect, base: Color32) {
    let inner = rect.shrink(2.0);
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, darken(base, 0.72)), egui::StrokeKind::Inside);
    painter.rect_stroke(inner, 2.0, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
}

fn print_sideways(painter: &egui::Painter, x: f32, y_center: f32, text: &str, size: f32, color: Color32) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as f32;
    let line_h = size * 1.2;
    for (i, ch) in chars.iter().enumerate() {
        let y = y_center + (i as f32 - (n - 1.0) / 2.0) * line_h;
        painter.text(
            egui::pos2(x, y),
            egui::Align2::CENTER_CENTER,
            ch.to_string(),
            egui::FontId::monospace(size),
            color,
        );
    }
}

fn paint_title_gradient(painter: &egui::Painter, rect: egui::Rect, accent: Color32, bg: Color32) {
    let top = darken(accent, 0.55);
    let deep = darken(accent, 0.82);
    let bands = 14;
    let mut prev_y = rect.min.y;
    for i in 0..bands {
        let t = i as f32 / bands as f32;
        let c = mix(mix(top, deep, t), bg, t * 0.85);
        let y = rect.min.y + (rect.max.y - rect.min.y) * t;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.min.x, prev_y), egui::pos2(rect.max.x, y)),
            0.0,
            c,
        );
        prev_y = y;
    }
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.max.y - 1.0),
            egui::pos2(rect.max.x, rect.max.y),
        ),
        0.0,
        accent,
    );
}

fn palette(name: &str) -> ThemePalette {
    match name {
        "Winamp" => ThemePalette {
            bg: hex("#0b0b0b"), fg: hex("#c8c8c8"), accent: hex("#1ce0ff"), highlight: hex("#1a2525"),
            btn_bg: hex("#565656"), btn_fg: hex("#e8e8e8"), playing_fg: hex("#8cff1a"), art_bg: hex("#000000"),
        },
        "Cyberpunk" => ThemePalette {
            bg: hex("#0d0221"), fg: hex("#ff2a6d"), accent: hex("#05d9e8"), highlight: hex("#1a1a3a"),
            btn_bg: hex("#1a0a2e"), btn_fg: hex("#ff2a6d"), playing_fg: hex("#05d9e8"), art_bg: hex("#1a0030"),
        },
        "Amber" => ThemePalette {
            bg: hex("#1a1400"), fg: hex("#ffb000"), accent: hex("#cc8800"), highlight: hex("#332200"),
            btn_bg: hex("#2a2000"), btn_fg: hex("#ffb000"), playing_fg: hex("#ffdd00"), art_bg: hex("#221100"),
        },
        "Ocean" => ThemePalette {
            bg: hex("#0a1628"), fg: hex("#00d4ff"), accent: hex("#0088aa"), highlight: hex("#0f2844"),
            btn_bg: hex("#122a4a"), btn_fg: hex("#00d4ff"), playing_fg: hex("#00ffff"), art_bg: hex("#061020"),
        },
        _ => ThemePalette {
            bg: hex("#1e1e1e"), fg: hex("#00ff00"), accent: hex("#00aa00"), highlight: hex("#003300"),
            btn_bg: hex("#2a2a2a"), btn_fg: hex("#00ff00"), playing_fg: hex("#00ff00"), art_bg: hex("#002200"),
        },
    }
}

const EQ_BANDS: [f64; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

const EQ_Q: f64 = 1.3;

struct EqPreset {
    name: &'static str,
    bands: [f32; 10],
}

const EQ_PRESETS: [EqPreset; 10] = [
    EqPreset { name: "Flat", bands: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { name: "Metal", bands: [5.0, 4.0, 2.0, -1.0, 3.0, 5.0, 6.0, 5.0, 4.0, 3.0] },
    EqPreset { name: "Rock", bands: [4.0, 3.0, 2.0, 1.0, 2.0, 4.0, 5.0, 4.0, 3.0, 2.0] },
    EqPreset { name: "Pop", bands: [-2.0, 0.0, 3.0, 5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0] },
    EqPreset { name: "Heavy Metal", bands: [6.0, 5.0, 3.0, -2.0, 2.0, 6.0, 7.0, 6.0, 5.0, 4.0] },
    EqPreset { name: "Death Metal", bands: [7.0, 6.0, 4.0, -3.0, 1.0, 7.0, 8.0, 6.0, 5.0, 4.0] },
    EqPreset { name: "Punk Rock", bands: [4.0, 2.0, 1.0, 0.0, 2.0, 4.0, 5.0, 4.0, 3.0, 2.0] },
    EqPreset { name: "Bass Boost", bands: [8.0, 6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { name: "Treble Boost", bands: [0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 6.0, 8.0, 9.0, 10.0] },
    EqPreset { name: "Vocal", bands: [-2.0, -1.0, 2.0, 4.0, 5.0, 4.0, 2.0, 1.0, 0.0, 0.0] },
];

static BEAUTIFY_RE: [std::sync::OnceLock<regex::Regex>; 4] = [
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeckMode {
    Tape,
    Disc,
    Radio,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum VideoAspect {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "4:3")]
    R43,
    #[serde(rename = "16:9")]
    R169,
}

impl VideoAspect {
    fn label(self) -> &'static str {
        match self {
            VideoAspect::Original => "ORIG",
            VideoAspect::R43 => "4:3",
            VideoAspect::R169 => "16:9",
        }
    }
    fn next(self) -> VideoAspect {
        match self {
            VideoAspect::Original => VideoAspect::R43,
            VideoAspect::R43 => VideoAspect::R169,
            VideoAspect::R169 => VideoAspect::Original,
        }
    }
}

impl Default for VideoAspect {
    fn default() -> Self {
        VideoAspect::Original
    }
}

fn beautify_name(filename: &str) -> String {
    let r1 = BEAUTIFY_RE[0].get_or_init(|| regex::Regex::new(r"^(CD\s?\d+[-]?\s*|\d+[-.]?\s*)+").unwrap());
    let r2 = BEAUTIFY_RE[1].get_or_init(|| regex::Regex::new(r"\s*\([^)]*\)\s*").unwrap());
    let r3 = BEAUTIFY_RE[2].get_or_init(|| regex::Regex::new(r"[\[\(]?\d{3}\s?(kbps|kb)?[\)]?$").unwrap());
    let r4 = BEAUTIFY_RE[3].get_or_init(|| regex::Regex::new(r"\s*-\s*\d{4}\s*$").unwrap());
    let mut s = r1.replace(filename, "").to_string();
    s = r2.replace(&s, "").to_string();
    s = r3.replace(&s, "").to_string();
    s = r4.replace(&s, "").to_string();
    let s = s.trim().to_string();
    if s.is_empty() { filename.to_string() } else { s }
}

fn is_audio(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_FORMATS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn fmt_secs(s: f32) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn fmt_freq(f: f32) -> String {
    if f >= 400.0 {
        format!("{} kHz", f.round())
    } else {
        format!("{:.1} FM", f)
    }
}

fn fmt_run(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn data_dir() -> PathBuf {
    let base = if let Ok(a) = std::env::var("APPDATA") {
        PathBuf::from(a)
    } else if let Ok(h) = std::env::var("USERPROFILE") {
        PathBuf::from(h)
    } else {
        PathBuf::from(".")
    };
    let d = base.join("MediaPlayerOFDOOM");
    let _ = std::fs::create_dir_all(&d);
    d
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Settings {
    volume: u32,
    last_played: Option<String>,
    playlist: Vec<String>,
    theme: String,
    #[serde(default)]
    video_aspects: HashMap<String, VideoAspect>,
    #[serde(default)]
    video_aspect_default: VideoAspect,
    #[serde(default = "default_video_volume")]
    video_volume: u32,
    #[serde(default)]
    last_cd_dir: Option<String>,
    #[serde(default)]
    video_positions: HashMap<String, f32>,
    #[serde(default)]
    intro_skip_enabled: bool,
    #[serde(default = "default_intro_skip")]
    intro_skip_secs: f32,
    #[serde(default = "default_credits_skip")]
    credits_skip_secs: f32,
    #[serde(default)]
    video_bounds: HashMap<String, (f32, f32)>,
    #[serde(default)]
    show_bounds: HashMap<String, (f32, f32)>,
}

fn default_video_volume() -> u32 {
    100
}

fn default_intro_skip() -> f32 {
    90.0
}

fn default_credits_skip() -> f32 {
    90.0
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RadioStation {
    name: String,
    band: String,
    freq: f32,
    url: String,
}

fn default_radio() -> Vec<RadioStation> {
    vec![
        RadioStation { name: "KEXP Indie/Rock".into(), band: "FM".into(), freq: 90.3, url: "http://live-mp3-128.kexp.org/kexp128.mp3".into() },
        RadioStation { name: "Radio Paradise".into(), band: "FM".into(), freq: 88.1, url: "https://stream.radioparadise.com/mp3-192".into() },
        RadioStation { name: "SomaFM Groove Salad".into(), band: "FM".into(), freq: 93.7, url: "https://ice1.somafm.com/groovesalad-128-mp3".into() },
        RadioStation { name: "SomaFM Indie Pop Rocks".into(), band: "FM".into(), freq: 94.5, url: "https://ice1.somafm.com/indiepop-128-mp3".into() },
        RadioStation { name: "SomaFM Sonic Universe".into(), band: "FM".into(), freq: 96.3, url: "https://ice1.somafm.com/sonicuniverse-128-mp3".into() },
        RadioStation { name: "SomaFM Underground 80s".into(), band: "FM".into(), freq: 101.1, url: "https://ice1.somafm.com/u80s-128-mp3".into() },
        RadioStation { name: "Kissin 92.5 (Joplin)".into(), band: "FM".into(), freq: 92.5, url: "https://playerservices.streamtheworld.com/api/livestream-redirect/KSYNFM.mp3".into() },
        RadioStation { name: "Big Dog 97.9 (Joplin)".into(), band: "FM".into(), freq: 97.9, url: "https://playerservices.streamtheworld.com/api/livestream-redirect/KXDGFM.mp3".into() },
        RadioStation { name: "Rock 107.1 (Joplin)".into(), band: "FM".into(), freq: 107.1, url: "https://ice42.securenetsystems.net/KJML".into() },
    ]
}

fn load_radio() -> Vec<RadioStation> {
    let p = data_dir().join("radio.json");
    if let Ok(text) = std::fs::read_to_string(&p) {
        if let Ok(list) = serde_json::from_str::<Vec<RadioStation>>(&text) {
            if !list.is_empty() {
                return list;
            }
        }
    }
    let list = default_radio();
    save_radio(&list);
    list
}

fn save_radio(list: &[RadioStation]) {
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(data_dir().join("radio.json"), text);
    }
}

struct PlayerState {
    volume: u32,
    current_song: Option<String>,
    playlist: Vec<String>,
    prev_songs: Vec<String>,
    repeat_enabled: bool,
    playlist_only_mode: bool,
    playlist_only_sequential: bool,
    dir_sequential: bool,
    is_paused: bool,
    song_count: u64,
    skip_count: u64,
    start_time: Option<Instant>,
    current_dir: Option<String>,
    theme: String,
    song_length: Duration,
    play_history: HashMap<String, u32>,
    song_weights: HashMap<String, f32>,
    eq_preset: String,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            volume: 50,
            theme: "Winamp".to_string(),
            eq_preset: "Flat".to_string(),
            current_song: None,
            playlist: Vec::new(),
            prev_songs: Vec::new(),
            repeat_enabled: false,
            playlist_only_mode: false,
            playlist_only_sequential: false,
            dir_sequential: false,
            is_paused: false,
            song_count: 0,
            skip_count: 0,
            start_time: None,
            current_dir: None,
            song_length: Duration::ZERO,
            play_history: HashMap::new(),
            song_weights: HashMap::new(),
        }
    }
}

struct Biquad {
    b0: f64, b1: f64, b2: f64, a1: f64, a2: f64,
    x1: f64, x2: f64, y1: f64, y2: f64,
}

impl Biquad {
    fn fresh(&self) -> Biquad {
        Biquad { b0: self.b0, b1: self.b1, b2: self.b2, a1: self.a1, a2: self.a2, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }
    fn bandpass_q(f0: f64, q: f64, fs: f64) -> Biquad {
        // RBJ cookbook peaking-Q band-pass, unity peak gain
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let c = w0.cos();
        let s = w0.sin();
        let alpha = s / (2.0 * q);
        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * c;
        let a2 = 1.0 - alpha;
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1; self.x1 = x;
        self.y2 = self.y1; self.y1 = y;
        y
    }
}

fn filter_signal(x: &[f64], coeffs: &Biquad) -> Vec<f64> {
    let mut f = coeffs.fresh();
    x.iter().map(|&v| f.process(v)).collect()
}

fn eq_out_path(rid: u64) -> PathBuf {
    std::env::temp_dir().join(format!("mpoofdoom_eq_{}.wav", rid))
}

fn gc_eq_files(current: u64) {
    if let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix("mpoofdoom_eq_") {
                if let Some(rest) = rest.strip_suffix(".wav") {
                    if let Ok(n) = rest.parse::<u64>() {
                        if n + 2 < current {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
            }
        }
    }
}

enum Msg {
    Error(String),
    DirScanned { dir: String, files: Vec<String> },
    PlaylistSaved,
    PlaylistLoaded { files: Vec<String> },
    Added { files: Vec<String> },
    FolderFound { folder: Option<String> },
    Meta { path: String, title: String, artist: String, album: String, duration: Duration },
    ArtLocal { bytes: Option<Vec<u8>> },
    ArtWeb { bytes: Option<Vec<u8>> },
    Lyrics { text: Option<String> },
    EqReady(u64),
    EqFailed(u64),
    YtStatus(String),
    YtLog(String),
    YtDone { path: PathBuf },
    YtFail(String),
    Transcoded { display: String, wav: PathBuf },
    TranscodeFail { display: String, err: String },
    Recorded { display: String, path: String },
    RecordFail { display: String, err: String },
    VideoFrame { w: u32, h: u32, rgba: Vec<u8> },
    VideoClosed { gen: u64 },
    VideoPos { gen: u64, secs: f32 },
    VideoMeta { gen: u64, dur: f32 },
    VideoBounds { gen: u64, path: String, intro_end: f32, credits_start: f32 },
}

enum LibCmd {
    Scan(String),
    AddMany(Vec<String>),
    LoadPlaylist(String),
    SavePlaylist(String, Vec<String>),
    Meta(String),
    ArtLocal(String),
    EqEncode(u64, String, Vec<f32>),
    Transcode { display: String, audio: String },
    Record { display: String, audio: String, dest: String },
    RecordRadio { display: String, url: String, dest: String },
    FindMusicFolder,
    VideoOpen { path: String, gen: u64, seek: f32 },
    VideoClose,
    VideoPause(bool),
    VideoAnalyze { path: String, gen: u64 },
}

enum NetCmd {
    WebArt(String, String),
    Lyrics(String, String),
    YtDownload(String),
}

fn collect_audio(dir: &str) -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>) {
        let rd = match std::fs::read_dir(d) { Ok(rd) => rd, Err(_) => return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if is_audio(&p.to_string_lossy()) {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(Path::new(dir), &mut out);
    out.sort();
    out
}

fn transcode_with_ffmpeg(audio: &str, ffmpeg: &Path) -> Result<PathBuf, String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    audio.hash(&mut h);
    let slug = format!("{:016x}", h.finish());
    let dir = std::env::temp_dir().join("mediaplayerofdoom_tc");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(format!("{}.wav", slug));
    if out.exists() && out.metadata().map(|m| m.len() > 4096).unwrap_or(false) {
        return Ok(out);
    }
    let ff = ffmpeg.to_string_lossy().to_string();
    let po = out.to_string_lossy().to_string();
    let st = std::process::Command::new(ff)
        .args(["-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && out.exists() {
        Ok(out)
    } else {
        Err("ffmpeg could not convert this file".into())
    }
}

fn record_with_ffmpeg(audio: &str, ffmpeg: &Path, dest: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let st = std::process::Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this file".into())
    }
}

fn record_flight_ffmpeg(url: &str, ffmpeg: &Path, dest: &Path, secs: u32) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let secs = secs.to_string();
    let st = std::process::Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-y", "-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", "-t", &secs, &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this stream".into())
    }
}

struct PipeSource {
    reader: BufReader<ChildStdout>,
    pending: VecDeque<i16>,
}

impl PipeSource {
    fn fill(&mut self) -> Option<()> {
        let mut chunk = [0u8; 4096];
        let n = self.reader.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        let n = n - (n % 2);
        let mut i = 0;
        while i < n {
            self.pending.push_back(i16::from_le_bytes([chunk[i], chunk[i + 1]]));
            i += 2;
        }
        Some(())
    }
}

impl Iterator for PipeSource {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        if self.pending.is_empty() {
            self.fill()?;
        }
        self.pending.pop_front()
    }
}

impl rodio::Source for PipeSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn spawn_radio_stream(url: &str, ffmpeg: &Path) -> Result<(Child, PipeSource), String> {
    let mut child = Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

fn spawn_video_audio(path: &str, ffmpeg: &Path, seek: f32) -> Result<(Child, PipeSource), String> {
    let mut args: Vec<String> = vec!["-re".to_string()];
    if seek > 0.0 {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", seek));
    }
    args.extend(["-i", path, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"].iter().map(|s| s.to_string()));
    let mut child = Command::new(ffmpeg.to_string_lossy().to_string())
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("ffmpeg video audio failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no audio output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

fn probe_video_duration(path: &str) -> f32 {
    let fp = tools_dir().join("ffprobe.exe");
    if !fp.is_file() {
        return 0.0;
    }
    let out = std::process::Command::new(fp)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(path)
        .creation_flags(NO_WINDOW)
        .output();
    match out {
        Ok(o) => std::str::from_utf8(&o.stdout).ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(0.0),
        Err(_) => 0.0,
    }
}

fn probe_video_info(path: &str) -> (u32, u32, f32) {
    let fp = tools_dir().join("ffprobe.exe");
    if !fp.is_file() {
        return (0, 0, 0.0);
    }
    let out = std::process::Command::new(fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate", "-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(path)
        .creation_flags(NO_WINDOW)
        .output();
    match out {
        Ok(o) => {
            let lines: Vec<&str> = std::str::from_utf8(&o.stdout).unwrap_or("")
                .lines().map(|s| s.trim()).collect();
            let w = lines.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let h = lines.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let fps = lines.get(2).and_then(|s| {
                if let Some((n, d)) = s.split_once('/') {
                    let n: f32 = n.trim().parse().ok()?;
                    let d: f32 = d.trim().parse().ok()?;
                    if d > 0.0 { Some(n / d) } else { None }
                } else {
                    s.parse::<f32>().ok().filter(|f| *f > 0.0)
                }
            }).unwrap_or(0.0);
            if w == 0 || h == 0 { (0, 0, fps) } else { (w, h, fps) }
        }
        Err(_) => (0, 0, 0.0),
    }
}

fn fit_video_dims(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 {
        return (640, 360);
    }
    let mut tw = w;
    let mut th = h;
    let ws = max_w as f32 / tw as f32;
    let hs = max_h as f32 / th as f32;
    let s = ws.min(hs);
    if s < 1.0 {
        tw = ((tw as f32) * s) as u32;
        th = ((th as f32) * s) as u32;
    }
    if tw % 2 != 0 { tw = tw.saturating_sub(1); }
    if th % 2 != 0 { th = th.saturating_sub(1); }
    (tw.max(2), th.max(2))
}

fn video_audio_levels(path: &str, ffmpeg: &Path, start: f32, len: f32) -> Vec<f32> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    if start > 0.0 {
        cmd.args(["-ss"]).arg(format!("{:.2}", start));
    }
    cmd.arg("-i").arg(path)
        .args(["-t"]).arg(format!("{:.2}", len))
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(NO_WINDOW);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let Some(mut stdout) = child.stdout.take() else { return vec![] };
    let mut out: Vec<f32> = Vec::new();
    let mut buf = vec![0u8; 8000];
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            match stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(_) => break,
            }
        }
        if filled < 2 {
            break;
        }
        let samples = filled / 2;
        let mut sum = 0.0f64;
        for i in 0..samples {
            let s = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f64 / 32768.0;
            sum += s * s;
        }
        let rms = (sum / samples as f64).sqrt();
        let db = if rms > 1e-9 { 20.0 * rms.log10() } else { -120.0 };
        out.push(db as f32);
        if filled < buf.len() {
            break;
        }
    }
    let _ = child.wait();
    out
}

fn smooth_curve(v: &[f32], w: usize) -> Vec<f32> {
    let mut s = vec![0.0; v.len()];
    for i in 0..v.len() {
        let lo = i.saturating_sub(w);
        let hi = (i + w).min(v.len());
        let cnt = hi - lo;
        if cnt > 0 {
            s[i] = v[lo..hi].iter().sum::<f32>() / cnt as f32;
        }
    }
    s
}

fn median_of(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

fn detect_intro_end(levels: &[f32], secs_per_sample: f32) -> f32 {
    if levels.len() < 40 {
        return 0.0;
    }
    let n = (levels.len() as f32 * 0.6).min(levels.len() as f32) as usize;
    let sm = smooth_curve(&levels[..n], 4);
    let med = median_of(&sm);
    let active = med + 3.5;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < sm.len() {
        if sm[i] >= active {
            let s = i;
            while i < sm.len() && sm[i] >= active {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in runs {
        if let Some(last) = merged.last_mut() {
            if s as isize - last.1 as isize <= 4 {
                last.1 = e;
                continue;
            }
        }
        merged.push((s, e));
    }
    for (s, e) in merged {
        let len_secs = (e - s) as f32 * secs_per_sample;
        let end_secs = e as f32 * secs_per_sample;
        if len_secs >= 15.0 && end_secs >= 35.0 {
            return end_secs;
        }
    }
    0.0
}

fn detect_credit_start(slice: &[f32], secs_per_sample: f32) -> f32 {
    if slice.len() < 30 {
        return 0.0;
    }
    let sm = smooth_curve(slice, 4);
    let med = median_of(&sm);
    let active = med + 3.0;
    let mut i = sm.len();
    while i > 0 {
        if sm[i - 1] >= active {
            let mut s = i - 1;
            while s > 0 && sm[s - 1] >= active {
                s -= 1;
            }
            let len = (i - s) as f32 * secs_per_sample;
            if len >= 20.0 {
                return s as f32 * secs_per_sample;
            }
            i = s;
        } else {
            i -= 1;
        }
    }
    0.0
}

fn fmt_time(s: f32) -> String {
    let s = s.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn lib_loop(rx: Receiver<LibCmd>, tx: Sender<Msg>) {
    let video_pause = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut video_child: Option<Child> = None;
    let mut video_thread: Option<std::thread::JoinHandle<()>> = None;
    if let Err(e) = ensure_tools(&tx) {
        let _ = tx.send(Msg::YtStatus(format!("Tools unavailable: {}", e)));
    }
    while let Ok(cmd) = rx.recv() {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle_lib(cmd, &tx, &video_pause, &mut video_child, &mut video_thread);
        }));
        if let Err(p) = res {
            let msg = if let Some(s) = p.downcast_ref::<&str>() { (*s).to_string() }
                else if let Some(s) = p.downcast_ref::<String>() { s.clone() }
                else { "unknown panic".to_string() };
            let _ = tx.send(Msg::Error(format!("Background task crashed: {}", msg)));
        }
    }
}

fn handle_lib(cmd: LibCmd, tx: &Sender<Msg>, video_pause: &std::sync::Arc<std::sync::atomic::AtomicBool>, video_child: &mut Option<Child>, video_thread: &mut Option<std::thread::JoinHandle<()>>) {
        match cmd {
            LibCmd::Scan(dir) => {
                let files = collect_audio(&dir);
                let _ = tx.send(Msg::DirScanned { dir, files });
            }
            LibCmd::AddMany(entries) => {
                let mut added: Vec<String> = Vec::new();
                for e in entries {
                    let p = PathBuf::from(&e);
                    if p.is_dir() {
                        let sub = collect_audio(&e);
                        for f in sub {
                            if !added.contains(&f) { added.push(f); }
                        }
                    } else if is_audio(&e) {
                        if !added.contains(&e) { added.push(e); }
                    }
                }
                added.sort();
                let _ = tx.send(Msg::Added { files: added });
            }
            LibCmd::LoadPlaylist(path) => {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        match serde_json::from_str::<Vec<String>>(&text) {
                            Ok(mut files) => {
                                files.sort();
                                let _ = tx.send(Msg::PlaylistLoaded { files });
                            }
                            Err(_) => { let _ = tx.send(Msg::Error("Invalid playlist file".into())); }
                        }
                    }
                    Err(_) => { let _ = tx.send(Msg::Error("Could not read playlist".into())); }
                }
            }
            LibCmd::SavePlaylist(path, playlist) => {
                let ok = serde_json::to_string(&playlist)
                    .map(|s| std::fs::write(&path, s).is_ok())
                    .unwrap_or(false);
                if ok { let _ = tx.send(Msg::PlaylistSaved); }
                else { let _ = tx.send(Msg::Error("Could not save playlist".into())); }
            }
            LibCmd::Meta(path) => {
                let mut title = stem(&path);
                let mut artist = "Unknown".to_string();
                let mut album = "Unknown".to_string();
                let mut duration = Duration::ZERO;
                if let Some(probed) = load_tagged(&path) {
                    duration = probed.properties().duration();
                    if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
                        if let Some(t) = tag.title() { title = t.to_string(); }
                        if let Some(a) = tag.artist() { artist = a.to_string(); }
                        if let Some(al) = tag.album() { album = al.to_string(); }
                    }
                }
                let _ = tx.send(Msg::Meta { path, title, artist, album, duration });
            }
            LibCmd::ArtLocal(path) => {
                let bytes = find_local_art(&path);
                let _ = tx.send(Msg::ArtLocal { bytes });
            }
            LibCmd::EqEncode(rid, path, bands) => {
                let bands_arr: [f32; 10] = bands.as_slice().try_into().unwrap_or([0.0; 10]);
                let out = eq_out_path(rid);
                match encode_eq(&path, &bands_arr, &out) {
                    Ok(()) => { let _ = tx.send(Msg::EqReady(rid)); }
                    Err(e) => {
                        let _ = tx.send(Msg::Error(format!("EQ failed: {}", e)));
                        let _ = tx.send(Msg::EqFailed(rid));
                    }
                }
            }
            LibCmd::Transcode { display, audio } => {
                match transcode_with_ffmpeg(&audio, &ffmpeg_path()) {
                    Ok(wav) => { let _ = tx.send(Msg::Transcoded { display, wav }); }
                    Err(e) => { let _ = tx.send(Msg::TranscodeFail { display, err: e }); }
                }
            }
            LibCmd::Record { display, audio, dest } => {
                match record_with_ffmpeg(&audio, &ffmpeg_path(), &Path::new(&dest)) {
                    Ok(p) => { let _ = tx.send(Msg::Recorded { display, path: p.to_string_lossy().to_string() }); }
                    Err(e) => { let _ = tx.send(Msg::RecordFail { display, err: e }); }
                }
            }
            LibCmd::RecordRadio { display, url, dest } => {
                match record_flight_ffmpeg(&url, &ffmpeg_path(), &Path::new(&dest), 30) {
                    Ok(p) => { let _ = tx.send(Msg::Recorded { display, path: p.to_string_lossy().to_string() }); }
                    Err(e) => { let _ = tx.send(Msg::RecordFail { display, err: e }); }
                }
            }
            LibCmd::FindMusicFolder => {
                let folder = find_music_folder();
                let _ = tx.send(Msg::FolderFound { folder });
            }
            LibCmd::VideoOpen { path, gen, seek } => {
                if let Some(mut c) = video_child.take() { let _ = c.kill(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                let (sw, sh, sfps) = probe_video_info(&path);
                let (tw, th) = fit_video_dims(sw, sh, 1920, 1080);
                let fps = if sfps > 0.0 { sfps } else { 24.0 };
                let mut cmd = std::process::Command::new(ffmpeg_path());
                cmd.args(["-hide_banner", "-loglevel", "error"]);
                if seek > 0.0 {
                    cmd.args(["-ss"]).arg(format!("{:.3}", seek));
                }
                cmd.arg("-i").arg(&path)
                    .args(["-an", "-vf", &format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2", tw, th, tw, th), "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(NO_WINDOW);
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Msg::Error(format!("ffmpeg could not start video: {}", e)));
                        return;
                    }
                };
                let dur = probe_video_duration(&path);
                let _ = tx.send(Msg::VideoMeta { gen, dur });
                if let Some(stdout) = child.stdout.take() {
                    let w = tw;
                    let h = th;
                    let pausef = std::sync::Arc::clone(video_pause);
                    let tx2 = tx.clone();
                    let pos_send = (fps as u64).max(1);
                    *video_thread = Some(std::thread::spawn(move || {
                        use std::io::Read;
                        let frame = (w * h * 3) as usize;
                        let mut buf = vec![0u8; frame];
                        let mut pipe = stdout;
                        let fps = fps;
                        let frame_t = std::time::Duration::from_secs_f64(1.0 / fps as f64);
                        let mut next_t = std::time::Instant::now();
                        let mut was_paused = false;
                        let mut frames: u64 = 0;
                        loop {
                            let paused = pausef.load(std::sync::atomic::Ordering::Relaxed);
                            if paused {
                                was_paused = true;
                                std::thread::sleep(std::time::Duration::from_millis(30));
                                continue;
                            }
                            if was_paused {
                                next_t = std::time::Instant::now();
                                was_paused = false;
                            }
                            let now = std::time::Instant::now();
                            if now < next_t {
                                std::thread::sleep(next_t - now);
                                continue;
                            }
                            if pipe.read_exact(&mut buf).is_err() {
                                break;
                            }
                            let mut rgba = Vec::with_capacity(frame / 3 * 4);
                            for p in buf.chunks_exact(3) {
                                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                            }
                            if tx2.send(Msg::VideoFrame { w, h, rgba }).is_err() {
                                break;
                            }
                            frames += 1;
                            if frames % pos_send == 0 {
                                let secs = seek + frames as f32 / fps;
                                if tx2.send(Msg::VideoPos { gen, secs }).is_err() {
                                    break;
                                }
                            }
                            next_t += frame_t;
                        }
                        let _ = tx2.send(Msg::VideoClosed { gen });
                    }));
                }
                *video_child = Some(child);
            }
            LibCmd::VideoClose => {
                if let Some(mut c) = video_child.take() { let _ = c.kill(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                let _ = tx.send(Msg::VideoClosed { gen: 0 });
            }
            LibCmd::VideoPause(p) => {
                video_pause.store(p, std::sync::atomic::Ordering::Relaxed);
            }
            LibCmd::VideoAnalyze { path, gen } => {
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let dur = probe_video_duration(&path);
                    if dur <= 0.0 {
                        let _ = tx2.send(Msg::VideoBounds { gen, path, intro_end: 0.0, credits_start: 0.0 });
                        return;
                    }
                    let ff = ffmpeg_path();
                    let front_len = dur.min(420.0);
                    let front = video_audio_levels(&path, &ff, 0.0, front_len);
                    let mut intro = detect_intro_end(&front, 0.5);
                    let mut credits = 0.0f32;
                    if dur > 240.0 {
                        let back_len = dur.min(420.0);
                        let back_start = (dur - back_len).max(0.0);
                        let back = video_audio_levels(&path, &ff, back_start, back_len);
                        let rel = detect_credit_start(&back, 0.5);
                        if rel > 0.0 {
                            credits = back_start + rel;
                        }
                    }
                    if intro >= dur - 5.0 {
                        intro = 0.0;
                    }
                    if credits > 0.0 && credits > dur - 3.0 {
                        credits = 0.0;
                    }
                    let _ = tx2.send(Msg::VideoBounds { gen, path, intro_end: intro, credits_start: credits });
                });
            }
        }
}

fn find_music_folder() -> Option<String> {
    let mut best: Option<(PathBuf, usize)> = None;
    let mut check = |d: &Path, prio: usize| {
        let files = collect_audio(&d.to_string_lossy());
        if !files.is_empty() {
            let score = files.len() + prio;
            if best.as_ref().map_or(true, |(_, s)| score > *s) {
                best = Some((d.to_path_buf(), score));
            }
        }
    };
    if let Ok(h) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(&h);
        check(&home.join("Music"), 100);
        check(&home.join("OneDrive").join("Music"), 100);
    }
    for root in existing_drive_roots() {
        check(&root.join("Music"), 40);
    }
    best.map(|(p, _)| p.to_string_lossy().to_string())
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
}

fn existing_drive_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        let mask = unsafe { GetLogicalDrives() };
        for bit in 0..26u32 {
            if mask & (1 << bit) != 0 {
                let ch = char::from_u32(b'A' as u32 + bit).unwrap_or('A');
                roots.push(PathBuf::from(format!("{}:\\", ch)));
            }
        }
    }
    #[cfg(not(windows))]
    {
        roots.push(PathBuf::from("/"));
    }
    roots
}

fn find_local_art(song_path: &str) -> Option<Vec<u8>> {
    let p = Path::new(song_path);
    if let Some(dir) = p.parent() {
        for name in ["cover.jpg", "cover.png", "folder.jpg", "album.jpg", "front.jpg", "default.jpg"] {
            let p2 = dir.join(name);
            if p2.is_file() {
                if let Ok(b) = std::fs::read(&p2) { return Some(b); }
            }
        }
        let mut imgs: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let ep = e.path();
                if let Some(ext) = ep.extension().and_then(|x| x.to_str()) {
                    if ["jpg", "jpeg", "png"].contains(&ext.to_lowercase().as_str()) {
                        imgs.push(ep);
                    }
                }
            }
        }
        imgs.sort();
        for ip in imgs {
            if let Ok(b) = std::fs::read(&ip) { return Some(b); }
        }
    }
    if let Some(probed) = load_tagged(song_path) {
        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                return Some(pic.data().to_vec());
            }
        }
    }
    None
}

fn load_tagged(path: &str) -> Option<lofty::file::TaggedFile> {
    let probe = lofty::probe::Probe::open(path).ok()?;
    let probe = probe.guess_file_type().ok()?;
    probe.read().ok()
}

fn encode_eq(path: &str, bands: &[f32; 10], out: &Path) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode_eq_impl(path, bands, out))) {
        Ok(r) => r,
        Err(_) => Err("EQ render panicked: file not decodable".into()),
    }
}

fn encode_eq_impl(path: &str, bands: &[f32; 10], out: &Path) -> Result<(), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe: {}", e))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| format!("decoder: {}", e))?;

    let mut left: Vec<f64> = Vec::new();
    let mut right: Vec<f64> = Vec::new();
    let mut rate: u32 = 0;
    let mut channels: usize = 0;
    let mut framed = false;

    loop {
        match format.next_packet() {
            Ok(packet) => {
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let frames = decoded.frames();
                        let mut sbuf = SampleBuffer::<f32>::new(frames as u64, spec);
                        sbuf.copy_interleaved_ref(decoded);
                        let data = sbuf.samples();
                        if data.is_empty() { continue; }
                        rate = spec.rate;
                        channels = spec.channels.count();
                        framed = true;
                        if channels <= 1 {
                            for &s in data {
                                left.push(s as f64);
                            }
                        } else {
                            let per = data.len() / channels;
                            for frame in 0..per {
                                left.push(data[frame * channels] as f64);
                                right.push(data[frame * channels + 1] as f64);
                            }
                        }
                    }
                    Err(_) => continue,
                }
            }
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(_) => break,
        }
    }
    if !framed || left.is_empty() {
        return Err("no audio decoded".into());
    }

    let nyquist = rate as f64 / 2.0;
    let fs = rate as f64;
    let band_gains: Vec<(f64, f64)> = EQ_BANDS
        .iter()
        .enumerate()
        .filter_map(|(i, f0)| {
            let db = bands[i] as f64;
            if db.abs() < 0.5 {
                return None;
            }
            let freq = (*f0).min(nyquist - 100.0).max(40.0);
            let g = db.signum() * (10.0_f64.powf(db.abs() / 20.0) - 1.0);
            Some((freq, g))
        })
        .collect();
    let mixed: Vec<(f64, Vec<f64>)> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        let lref = &left;
        for (freq, gain) in band_gains {
            handles.push(s.spawn(move || {
                let f = Biquad::bandpass_q(freq, EQ_Q, fs);
                let filt = filter_signal(lref, &f);
                (gain, filt)
            }));
        }
        handles.into_iter().map(|h| h.join().expect("eq band thread")).collect()
    });
    for (gain, fv) in mixed {
        for (j, fv) in fv.iter().enumerate() {
            left[j] += gain * fv;
            if channels > 1 {
                right[j] += gain * fv;
            }
        }
    }

    let _ = std::fs::remove_file(out);
    let wspec = hound::WavSpec {
        channels: if channels > 1 { 2 } else { 1 } as u16,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let f = File::create(out).map_err(|e| e.to_string())?;
    let mut wr = hound::WavWriter::new(f, wspec).map_err(|e| e.to_string())?;
    let to_i16 = |v: f64| (v.clamp(-1.0, 1.0) * 32767.0) as i16;
    if channels > 1 {
        for j in 0..left.len() {
            let _ = wr.write_sample(to_i16(left[j]));
            let _ = wr.write_sample(to_i16(right[j]));
        }
    } else {
        for v in &left {
            let _ = wr.write_sample(to_i16(*v));
        }
    }
    wr.finalize().map_err(|e| e.to_string())?;
    Ok(())
}

fn net_loop(rx: Receiver<NetCmd>, tx: Sender<Msg>) {
    while let Ok(cmd) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                NetCmd::WebArt(artist, album) => {
                    let bytes = fetch_web_art(&artist, &album);
                    let _ = tx.send(Msg::ArtWeb { bytes });
                }
                NetCmd::Lyrics(artist, title) => {
                    let text = fetch_lyrics(&artist, &title);
                    let _ = tx.send(Msg::Lyrics { text });
                }
                NetCmd::YtDownload(query) => {
                    let r = yt_download_and_store(&query, &tx);
                    match r {
                        Ok(path) => { let _ = tx.send(Msg::YtDone { path }); }
                        Err(e) => { let _ = tx.send(Msg::YtFail(e)); }
                    }
                }
            }
        }));
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_' || *b == b'.' || *b == b'~' {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn fetch_web_art(artist: &str, album: &str) -> Option<Vec<u8>> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(6)).build().ok()?;
    let res = client
        .get("https://itunes.apple.com/search")
        .query(&[("term", format!("{} {}", artist, album)), ("entity", "album".to_string()), ("limit", "1".to_string())])
        .send()
        .ok()?;
    let json: serde_json::Value = res.json().ok()?;
    let n = json.get("resultCount")?.as_u64()?;
    if n == 0 { return None; }
    let url = json["results"][0]["artworkUrl100"].as_str()?;
    let url = url.replace("100x100bb", "500x500bb");
    client.get(&url).send().ok()?.bytes().ok().map(|b| b.to_vec())
}

fn fetch_lyrics(artist: &str, title: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
    let url = format!("https://api.lyrics.ovh/v1/{}/{}", percent_encode(artist), percent_encode(title));
    let res = client.get(&url).send().ok()?;
    if !res.status().is_success() { return None; }
    let json: serde_json::Value = res.json().ok()?;
    json.get("lyrics").and_then(|l| l.as_str()).map(|s| s.to_string())
}

fn truncate_mid(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3) / 2;
    let head: String = s.chars().take(keep).collect();
    let tail: String = s.chars().rev().take(keep).collect::<Vec<char>>().into_iter().rev().collect();
    format!("{}...{}", head, tail)
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn tools_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| exe_dir());
    let d = base.join("MediaPlayerOFDOOM").join("tools");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn ytdlp_path() -> PathBuf {
    tools_dir().join("yt-dlp.exe")
}

fn ffmpeg_path() -> PathBuf {
    tools_dir().join("ffmpeg.exe")
}

fn music_folder() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| exe_dir());
    let p = base.join("Music");
    if !p.is_dir() {
        let _ = std::fs::create_dir_all(&p);
    }
    p
}

fn download_tool(url: &str, dst: &Path, label: &str, tx: &Sender<Msg>) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
    let _ = tx.send(Msg::YtStatus(format!("Downloading {}...", label)));
    let mut resp = client.get(url).send().map_err(|e| format!("{} download failed: {}", label, e))?;
    let mut out = File::create(dst).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_bin(dst: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    std::fs::write(dst, bytes).map_err(|e| format!("could not extract {}: {}", label, e))
}

fn ensure_tools(tx: &Sender<Msg>) -> Result<(), String> {
    let _ = std::fs::create_dir_all(tools_dir());
    let ytdlp = ytdlp_path();
    if !ytdlp.is_file() {
        match EMBEDDED_YTDLP {
            Some(b) => write_bin(&ytdlp, b, "yt-dlp")?,
            None => download_tool(
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
                &ytdlp,
                "yt-dlp",
                tx,
            )?,
        }
    }
    let ff = ffmpeg_path();
    let fp = tools_dir().join("ffprobe.exe");
    if !ff.is_file() {
        let _ = tx.send(Msg::YtStatus("Extracting ffmpeg...".into()));
        match EMBEDDED_FFMPEG {
            Some(b) => write_bin(&ff, b, "ffmpeg")?,
            None => {
                let _ = tx.send(Msg::YtStatus("Downloading ffmpeg...".into()));
                let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
                let resp = client.get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip").send().map_err(|e| format!("ffmpeg download failed: {}", e))?;
                let bytes = resp.bytes().map_err(|e| e.to_string())?;
                let _ = tx.send(Msg::YtStatus("Extracting ffmpeg...".into()));
                let mut cur = std::io::Cursor::new(bytes);
                let mut za = zip::ZipArchive::new(&mut cur).map_err(|e| format!("bad ffmpeg archive: {}", e))?;
                for i in 0..za.len() {
                    let mut f = za.by_index(i).map_err(|e| e.to_string())?;
                    let name = f.name().replace('\\', "/");
                    let fname = name.rsplit('/').next().unwrap_or("").to_string();
                    if fname == "ffmpeg.exe" || fname == "ffprobe.exe" {
                        let target = tools_dir().join(&fname);
                        let mut out = File::create(&target).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
                    }
                }
                if !ff.is_file() {
                    return Err("ffmpeg extraction produced no ffmpeg.exe".into());
                }
            }
        }
    }
    if !fp.is_file() {
        match EMBEDDED_FFPROBE {
            Some(b) => write_bin(&fp, b, "ffprobe")?,
            None => {
                if !ff.is_file() {
                    return Err("ffprobe missing and not embedded; run online once".into());
                }
                let _ = tx.send(Msg::YtStatus("Extracting ffprobe...".into()));
                let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
                let resp = client.get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip").send().map_err(|e| format!("ffprobe download failed: {}", e))?;
                let bytes = resp.bytes().map_err(|e| e.to_string())?;
                let mut cur = std::io::Cursor::new(bytes);
                let mut za = zip::ZipArchive::new(&mut cur).map_err(|e| format!("bad ffmpeg archive: {}", e))?;
                for i in 0..za.len() {
                    let mut f = za.by_index(i).map_err(|e| e.to_string())?;
                    let name = f.name().replace('\\', "/");
                    let fname = name.rsplit('/').next().unwrap_or("").to_string();
                    if fname == "ffprobe.exe" {
                        let target = tools_dir().join(&fname);
                        let mut out = File::create(&target).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn sanitize_win(s: &str) -> String {
    let bad = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out: String = s.trim().chars().map(|c| if bad.contains(&c) { '_' } else { c }).collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out = "untitled".to_string();
    }
    out
}

fn next_track_num(dir: &Path) -> u32 {
    let mut max = 0u32;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = digits.parse::<u32>() {
                max = max.max(v);
            }
        }
    }
    max + 1
}

fn yt_lookup(yt: &Path, target: &str) -> Result<(String, String, String, String), String> {
    let out = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args(["--skip-download", "--no-warnings", "--newline", "--print", "%(id)s|%(title)s|%(artist)s|%(uploader)s", target])
        .output()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    let first = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string();
    if first.is_empty() {
        return Err(format!("yt-dlp returned no metadata (exit {})", out.status.code().unwrap_or(-1)));
    }
    let mut it = first.split('|');
    Ok((
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    ))
}

fn yt_grab(yt: &Path, ffmpeg: &Path, target: &str, id: &str, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    use std::io::BufRead;
    use std::process::Stdio;
    let tmp = std::env::temp_dir().join("mediaplayerofdoom_yt");
    let _ = std::fs::create_dir_all(&tmp);
    let tpl = tmp.join(format!("{}.%(ext)s", id));
    let _ = std::fs::remove_file(tmp.join(format!("{}.mp3", id)));
    let mut child = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args([
            "--newline",
            "--no-warnings",
            "--ffmpeg-location",
            ffmpeg.parent().and_then(|p| p.to_str()).unwrap_or(""),
            "-f", "bestaudio/best",
            "-x",
            "--audio-format", "mp3",
            "--audio-quality", "320k",
            "--embed-metadata",
            "--embed-thumbnail",
            "--convert-thumbnails", "jpg",
            "--parse-metadata", "%(uploader)s:%(artist)s",
            "-o", &tpl.to_string_lossy(),
            target,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    let mut last_err = String::new();
    if let Some(se) = child.stderr.take() {
        let mut reader = BufReader::new(se);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            let s = line.trim_end().to_string();
            if !s.is_empty() {
                last_err = s.clone();
                if is_yt_log_line(&s) {
                    let _ = tx.send(Msg::YtLog(s));
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let final_mp3 = tmp.join(format!("{}.mp3", id));
    if !status.success() || !final_mp3.is_file() {
        let tail: Vec<&str> = last_err.lines().rev().take(4).collect();
        let err = tail.join("\n");
        return Err(if err.is_empty() { format!("yt-dlp exited {}", status.code().unwrap_or(-1)) } else { err });
    }
    Ok(final_mp3)
}

fn yt_download_and_store(query: &str, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    ensure_tools(tx)?;
    let q = query.trim();
    if q.is_empty() {
        return Err("Enter a YouTube URL or search term".into());
    }
    let target = if q.starts_with("http") { q.to_string() } else { format!("ytsearch1:{}", q) };
    let (id, title, artist, uploader) = yt_lookup(&ytdlp_path(), &target)?;
    let mut display = title.clone();
    if display.is_empty() {
        display = id.clone();
    }
    let _ = tx.send(Msg::YtStatus(format!("Downloading \"{}\"...", display)));
    let mp3 = yt_grab(&ytdlp_path(), &ffmpeg_path(), &target, &id, tx)?;
    let folder = music_folder();
    let num = next_track_num(&folder);
    let artist = if artist.is_empty() { uploader } else { artist };
    let artist = if artist.is_empty() { "Unknown".to_string() } else { artist };
    let mut fname = format!("{:03} - {} ({}).mp3", num, sanitize_win(&title), sanitize_win(&artist));
    let mut final_path = folder.join(&fname);
    let mut extra = 0u32;
    while final_path.exists() {
        extra += 1;
        fname = format!("{:03}.{} - {} ({}).mp3", num, extra, sanitize_win(&title), sanitize_win(&artist));
        final_path = folder.join(&fname);
    }
    std::fs::rename(&mp3, &final_path).map_err(|e| format!("could not save to Music folder: {}", e))?;
    Ok(final_path)
}

struct PlayerApp {
    ctx: egui::Context,
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Sink,
    state: PlayerState,
    settings_path: PathBuf,
    history_path: PathBuf,
    lib_tx: Sender<LibCmd>,
    net_tx: Sender<NetCmd>,
    msg_rx: Receiver<Msg>,
    status: String,
    error: String,
    status_until: Option<Instant>,
    error_until: Option<Instant>,
    meta: (String, String, String),
    playing: String,
    art_tex: Option<TextureHandle>,
    art_state: u8,
    art_local_valid: bool,
    display_cache: Vec<String>,
    playlist_scroll: f32,
    pl_view_h: f32,
    playing_pl_idx: Option<usize>,
    search_query: String,
    yt_query: String,
    eq_pending: Option<String>,
    eq_resume: Option<Duration>,
    eq_req_id: u64,
    eq_dirty: bool,
    yt_log: Vec<String>,
    yt_pct: f32,
    yt_speed: String,
    yt_eta: String,
    last_skip: Instant,
    play_started: Option<Instant>,
    pos: f32,
    len_secs: f32,
    dragging: bool,
    running: String,
    fullscreen: bool,
    fs_asserted: bool,
    video_fs_restore_app: bool,
    tape_anim: f32,
    tape_out: bool,
    tape_reversing: bool,
    tape_timer: f32,
    tape_side: char,
    tape_songs: u32,
    tape_cap: u32,
    winding: Option<f32>,
    wind_arm: Option<(f32, f64)>,
    wind_playing: bool,
    recording: bool,
    last_time: f64,
    pending_transcode: Option<(String, String)>,
    transcodes: HashMap<String, String>,
    sleep_minutes: u32,
    sleep_input: String,
    sleep_deadline: Option<Instant>,
    fading: bool,
    fade_start: Instant,
    fade_from_vol: u32,
    show_help: bool,
    search_focus: bool,
    yt_focus: bool,
    eq_on: bool,
    eq_custom: Option<[f32; 10]>,
    lyrics: Option<String>,
    lyrics_title: String,
    viz_bars: Vec<f32>,
    viz_peaks: Vec<f32>,
    want_web: bool,
    last_drop: Vec<PathBuf>,
    drag_from: Option<usize>,
    drag_hover: Option<usize>,
    drag_active: bool,
    _tray: Option<TrayIcon>,
    _hkm: Option<GlobalHotKeyManager>,
    show_id: u32,
    quit_id: u32,
    radio_presets: Vec<RadioStation>,
    show_radio: bool,
    deck_mode: DeckMode,
    radio_freq: f32,
    radio_band: String,
    radio_tuned: Option<usize>,
    radio_on: bool,
    radio_playing_idx: Option<usize>,
    radio_loading: bool,
    radio_proc: Option<Child>,
    radio_last_moved: f64,
    radio_last_start: Instant,
    disc_in: bool,
    disc_label: String,
    last_cd_dir: Option<String>,
    disc_saved: Vec<String>,
    band_map: HashMap<String, Vec<String>>,
    full_library: Vec<String>,
    band_sel: String,
    bands_path: PathBuf,
    video_on: bool,
    video_paused: bool,
    video_dims: (u32, u32),
    video_tex: Option<egui::TextureHandle>,
    video_pos: egui::Pos2,
    video_fs: bool,
    video_sink: Option<Sink>,
    video_audio_proc: Option<Child>,
    video_audio_pending: bool,
    video_last_mouse: f64,
    video_bar_visible: bool,
    video_queue: Vec<String>,
    video_queue_idx: usize,
    video_gen: u64,
    video_closing: bool,
    video_aspect: VideoAspect,
    video_dur_secs: f32,
    video_cur_secs: f32,
    video_seek_t: Option<f32>,
    video_current: Option<String>,
    video_ended: bool,
    video_aspects: HashMap<String, VideoAspect>,
    video_aspect_default: VideoAspect,
    video_volume: u32,
    deck_resume: Option<(String, f32)>,
    video_positions: HashMap<String, f32>,
    intro_skip_enabled: bool,
    intro_skip_secs: f32,
    credits_skip_secs: f32,
    video_bounds: HashMap<String, (f32, f32)>,
    show_bounds: HashMap<String, (f32, f32)>,
    analyzing_video: HashSet<String>,
    random_on: bool,
    switch_accum: f32,
}

impl PlayerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let ctx = cc.egui_ctx.clone();

        let (stream, handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&handle)?;
        sink.pause();

        let settings_path = data_dir().join("settings.json");
        let history_path = data_dir().join("history.json");

        let mut state = PlayerState::default();
        let mut video_aspects: HashMap<String, VideoAspect> = HashMap::new();
        let mut video_aspect_default = VideoAspect::Original;
        let mut video_volume = 100;
        let mut last_cd_dir: Option<String> = None;
        let mut video_positions: HashMap<String, f32> = HashMap::new();
        let mut intro_skip_enabled = false;
        let mut intro_skip_secs: f32 = 90.0;
        let mut credits_skip_secs: f32 = 90.0;
        let mut video_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        let mut show_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(&settings_path) {
            if let Ok(s) = serde_json::from_str::<Settings>(&text) {
                state.volume = s.volume.clamp(0, 100);
                state.current_song = s.last_played.filter(|p| Path::new(p).is_file());
                state.playlist = s.playlist;
                state.playlist.sort_by_key(|p| song_sort_key(p));
                if !s.theme.is_empty() { state.theme = s.theme; }
                video_aspects = s.video_aspects;
                video_aspect_default = s.video_aspect_default;
                video_volume = s.video_volume.clamp(0, 200);
                last_cd_dir = s.last_cd_dir.filter(|d| Path::new(d).is_dir());
                video_positions = s.video_positions;
                intro_skip_enabled = s.intro_skip_enabled;
                intro_skip_secs = s.intro_skip_secs;
                credits_skip_secs = s.credits_skip_secs;
                video_bounds = s.video_bounds;
                show_bounds = s.show_bounds;
            }
        }
        if let Ok(text) = std::fs::read_to_string(&history_path) {
            if let Ok(h) = serde_json::from_str::<HashMap<String, u32>>(&text) {
                state.play_history = h;
            }
        }
        sink.set_volume(state.volume as f32 / 100.0);

        let (lib_tx, lib_rx) = channel::<LibCmd>();
        let (net_tx, net_rx) = channel::<NetCmd>();
        let (msg_tx, msg_rx) = channel::<Msg>();
        {
            let tx = msg_tx.clone();
            thread::spawn(move || lib_loop(lib_rx, tx));
        }
        {
            let tx = msg_tx.clone();
            thread::spawn(move || net_loop(net_rx, tx));
        }

        let display_cache: Vec<String> = state.playlist.iter().map(|p| make_display(p)).collect();

        let old_bands = data_dir().join("bands.json");
        if old_bands.exists() {
            let _ = std::fs::remove_file(&old_bands);
        }
        let old2 = data_dir().join("bands2.json");
        if old2.exists() {
            let _ = std::fs::remove_file(&old2);
        }
        let bands_path = data_dir().join("bands3.json");
        let band_map = load_bands(&bands_path);
        let full_library: Vec<String> = state.playlist.clone();

        let tray_config = make_tray().ok();
        let hotkeys = make_hotkeys();
        let (show_id, quit_id) = hotkeys.as_ref().map(|(_, s, q)| (*s, *q)).unwrap_or((0, 0));

        let app = Self {
            ctx: ctx.clone(),
            _stream: stream,
            _stream_handle: handle,
            sink,
            state,
            settings_path,
            history_path,
            lib_tx,
            net_tx,
            msg_rx,
            status: format!("{} v{} - ready", APP_NAME, APP_VERSION),
            error: String::new(),
            status_until: None,
            error_until: None,
            meta: (String::new(), "Unknown".to_string(), "Unknown".to_string()),
            playing: String::new(),
            art_tex: None,
            art_state: 0,
            art_local_valid: false,
            display_cache,
            playlist_scroll: 0.0,
            pl_view_h: 300.0,
            playing_pl_idx: None,
            search_query: String::new(),
            yt_query: String::new(),
            eq_pending: None,
            eq_resume: None,
            eq_req_id: 0,
            eq_dirty: false,
            yt_log: Vec::new(),
            yt_pct: -1.0,
            yt_speed: String::new(),
            yt_eta: String::new(),
            last_skip: Instant::now(),
            play_started: None,
            pos: 0.0,
            len_secs: 0.0,
            dragging: false,
            running: String::new(),
            fullscreen: true,
            fs_asserted: false,
            video_fs_restore_app: false,
            tape_anim: 0.0,
            tape_out: false,
            tape_reversing: false,
            tape_timer: 0.0,
            tape_side: 'A',
            tape_songs: 0,
            tape_cap: 5,
            winding: None,
            wind_arm: None,
            wind_playing: false,
            recording: false,
            radio_presets: load_radio(),
            show_radio: false,
            deck_mode: DeckMode::Tape,
            radio_freq: 88.1,
            radio_band: "FM".to_string(),
            radio_tuned: None,
            radio_on: false,
            radio_playing_idx: None,
            radio_loading: false,
            radio_proc: None,
            radio_last_moved: 0.0,
            radio_last_start: Instant::now(),
            disc_in: false,
            disc_label: String::new(),
            last_cd_dir,
            disc_saved: Vec::new(),
            band_map,
            full_library,
            band_sel: String::new(),
            video_on: false,
            video_paused: false,
            video_dims: (640, 360),
            video_tex: None,
            video_pos: egui::pos2(60.0, 60.0),
            video_fs: false,
            video_sink: None,
            video_audio_proc: None,
            video_audio_pending: false,
            video_last_mouse: 0.0,
            video_bar_visible: false,
            video_queue: Vec::new(),
            video_queue_idx: 0,
            video_gen: 0,
            video_closing: false,
            video_aspect: VideoAspect::Original,
            video_dur_secs: 0.0,
            video_cur_secs: 0.0,
            video_seek_t: None,
            video_current: None,
video_ended: false,
        video_aspects,
        video_aspect_default,
        video_volume,
        deck_resume: None,
        video_positions: HashMap::new(),
        intro_skip_enabled: false,
            intro_skip_secs: 90.0,
            credits_skip_secs: 90.0,
            video_bounds: HashMap::new(),
            show_bounds: HashMap::new(),
            analyzing_video: HashSet::new(),
            random_on: true,
            switch_accum: 0.0,
            bands_path,
            last_time: 0.0,
            pending_transcode: None,
            transcodes: HashMap::new(),
            sleep_minutes: 0,
            sleep_input: "0".to_string(),
            sleep_deadline: None,
            fading: false,
            fade_start: Instant::now(),
            fade_from_vol: 50,
            show_help: false,
            search_focus: false,
            yt_focus: false,
            eq_on: false,
            eq_custom: None,
            lyrics: None,
            lyrics_title: String::new(),
            viz_bars: vec![2.0; 12],
            viz_peaks: vec![0.0; 12],
            want_web: false,
            last_drop: Vec::new(),
            drag_from: None,
            drag_hover: None,
            drag_active: false,
            _tray: tray_config,
            _hkm: hotkeys.map(|(h, _, _)| h),
            show_id,
            quit_id,
        };
        let _ = app.lib_tx.send(LibCmd::FindMusicFolder);
        Ok(app)
    }

    fn save_settings(&self) {
        let s = Settings {
            volume: self.state.volume,
            last_played: self.state.current_song.clone(),
            playlist: self.state.playlist.clone(),
            theme: self.state.theme.clone(),
            video_aspects: self.video_aspects.clone(),
            video_aspect_default: self.video_aspect_default,
            video_volume: self.video_volume,
            last_cd_dir: self.last_cd_dir.clone(),
            video_positions: self.video_positions.clone(),
            intro_skip_enabled: self.intro_skip_enabled,
            intro_skip_secs: self.intro_skip_secs,
            credits_skip_secs: self.credits_skip_secs,
            video_bounds: self.video_bounds.clone(),
            show_bounds: self.show_bounds.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(&self.settings_path, text);
        }
    }

    fn save_history(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&self.state.play_history) {
            let _ = std::fs::write(&self.history_path, text);
        }
    }

    fn palette(&self) -> ThemePalette {
        palette(&self.state.theme)
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn set_error(&mut self, s: impl Into<String>) {
        self.error = s.into();
        self.error_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn theme_colors(&self) -> ThemePalette {
        self.palette()
    }

    fn apply_visuals(&self, ctx: &egui::Context) {
        let th = self.palette();
        let mut vis = egui::Visuals::dark();
        vis.panel_fill = th.bg;
        vis.window_fill = th.bg;
        vis.extreme_bg_color = th.highlight;
        vis.faint_bg_color = th.highlight;
        vis.hyperlink_color = th.accent;
        vis.override_text_color = Some(th.fg);
        vis.selection.bg_fill = th.accent;
        vis.selection.stroke = egui::Stroke::new(1.0, th.fg);
        vis.slider_trailing_fill = true;
        let set = |w: &mut egui::style::WidgetVisuals| {
            w.bg_fill = th.btn_bg;
            w.weak_bg_fill = th.highlight;
            w.fg_stroke = egui::Stroke::new(1.0, th.btn_fg);
            w.bg_stroke = egui::Stroke::new(1.0, th.highlight);
            w.corner_radius = egui::CornerRadius::same(2);
        };
        set(&mut vis.widgets.noninteractive);
        set(&mut vis.widgets.inactive);
        set(&mut vis.widgets.hovered);
        set(&mut vis.widgets.active);
        set(&mut vis.widgets.open);
        vis.widgets.hovered.bg_fill = th.highlight;
        vis.widgets.active.bg_fill = th.highlight;
        ctx.set_visuals(vis);
        let mut style = egui::Style::default();
        style.spacing.slider_width = 160.0;
        style.spacing.item_spacing = egui::vec2(5.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        ctx.set_style_of(egui::Theme::Dark, style);
    }

    fn rebuild_display_cache(&mut self) {
        self.display_cache = self.state.playlist.iter().map(|p| make_display(p)).collect();
    }

    fn playing_display(path: &str) -> String {
        let p = Path::new(path);
        let folder = p.parent().and_then(|x| x.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "?".to_string());
        format!("{} > {}", folder, beautify_name(&stem(path)))
    }

    fn eq_cur(&self) -> [f32; 10] {
        if let Some(c) = self.eq_custom {
            return c;
        }
        EQ_PRESETS.iter().find(|p| p.name == self.state.eq_preset).map(|p| p.bands).unwrap_or(EQ_PRESETS[0].bands)
    }

    fn eq_name(&self) -> &str {
        if self.eq_custom.is_some() { "Custom" } else { self.state.eq_preset.as_str() }
    }

    fn drain_msg(&mut self) {
        while let Ok(m) = self.msg_rx.try_recv() {
            match m {
                Msg::Error(s) => self.set_error(s),
                Msg::DirScanned { dir, files } => {
                    self.state.current_dir = Some(dir.clone());
                    self.ingest_dir(&dir, &files);
                    self.state.playlist = files;
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.state.song_count = 0;
                    self.state.skip_count = 0;
                    self.state.start_time = Some(Instant::now());
                    self.playing_pl_idx = None;
                    self.rebuild_display_cache();
                    if self.state.playlist.is_empty() {
                        self.set_status("No songs found");
                    } else {
                        self.set_status(format!("Loaded {} songs", self.state.playlist.len()));
                    }
                    self.save_settings();
                    if !self.state.playlist.is_empty() && !self.state.is_paused {
                        self.skip_song();
                    }
                }
                Msg::PlaylistSaved => self.set_status("Playlist saved"),
                Msg::PlaylistLoaded { files } => {
                    self.ingest_bands(&files, None);
                    self.state.playlist = files;
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.rebuild_display_cache();
                    self.set_status("Playlist loaded");
                    self.save_settings();
                }
                Msg::Added { files } => {
                    self.ingest_bands(&files, None);
                    let mut n = 0;
                    for f in files {
                        if !self.state.playlist.contains(&f) {
                            self.state.playlist.push(f);
                            n += 1;
                        }
                    }
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.rebuild_display_cache();
                    self.set_status(format!("Added {} files", n));
                    self.save_settings();
                }
                Msg::FolderFound { folder } => {
                    if let Some(f) = folder {
                        self.set_status(format!("Auto-loaded: {}", f));
                        let _ = self.lib_tx.send(LibCmd::Scan(f));
                    } else {
                        self.set_status("No Music folder found — use ADD DIR to pick one");
                    }
                }
                Msg::Meta { path, title, artist, album, duration } => {
                    if self.state.current_song.as_deref() == Some(&path) {
                        self.meta = (title.clone(), artist.clone(), album.clone());
                        self.state.song_length = duration;
                        self.len_secs = duration.as_secs_f32();
                        self.want_web = artist != "Unknown" && album != "Unknown";
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("♪ {} - {}", artist, title)));
                    }
                }
                Msg::ArtLocal { bytes } => {
                    if let Some(b) = bytes {
                        self.art_local_valid = true;
                        self.load_art_texture(b);
                    } else if self.want_web {
                        let (a, al) = (self.meta.1.clone(), self.meta.2.clone());
                        let _ = self.net_tx.send(NetCmd::WebArt(a, al));
                    }
                }
                Msg::ArtWeb { bytes } => {
                    if let Some(b) = bytes {
                        if self.want_web && !self.art_local_valid {
                            self.load_art_texture(b);
                        }
                    }
                }
                Msg::Lyrics { text } => {
                    if let Some(t) = text {
                        self.lyrics = Some(t);
                        self.lyrics_title = format!("{} - {}", self.meta.1, self.meta.0);
                    } else {
                        self.set_error("Lyrics not found");
                    }
                }
                Msg::EqReady(rid) => {
                    let current = rid == self.eq_req_id;
                    let was_dirty = self.eq_dirty;
                    if current {
                        let p = self.eq_pending.take();
                        self.eq_dirty = false;
                        if !was_dirty {
                            if let Some(p) = p {
                                if self.state.current_song.as_deref() == Some(p.as_str()) {
                                    self.swap_eq(&p);
                                } else {
                                    gc_eq_files(self.eq_req_id);
                                }
                            }
                        } else if let Some(p) = p {
                            gc_eq_files(self.eq_req_id);
                            self.restart_current();
                            let _ = p;
                        }
                    } else if self.eq_pending.is_none() && self.eq_dirty {
                        self.eq_dirty = false;
                        self.restart_current();
                    }
                }
                Msg::EqFailed(rid) => {
                    let current = rid == self.eq_req_id;
                    let was_dirty = self.eq_dirty;
                    if current {
                        if let Some(p) = self.eq_pending.take() {
                            self.eq_dirty = false;
                            self.set_error(format!("EQ failed for: {} (using original)", stem(&p)));
                        }
                        if was_dirty {
                            self.restart_current();
                        }
                    } else if self.eq_pending.is_none() && self.eq_dirty {
                        self.eq_dirty = false;
                        self.restart_current();
                    }
                }
                Msg::YtStatus(s) => {
                    self.status = s;
                    self.status_until = Some(Instant::now() + Duration::from_secs(8));
                }
                Msg::YtLog(s) => {
                    if let Some((p, sp, et)) = parse_yt_progress(&s) {
                        self.yt_pct = p;
                        self.yt_speed = sp;
                        self.yt_eta = et;
                    }
                    self.yt_log.push(s);
                    const MAX_LOG: usize = 60;
                    if self.yt_log.len() > MAX_LOG {
                        let over = self.yt_log.len() - MAX_LOG;
                        self.yt_log.drain(0..over);
                    }
                }
                Msg::YtDone { path } => {
                    self.yt_pct = -1.0;
                    self.yt_speed.clear();
                    self.yt_eta.clear();
                    let s = path.to_string_lossy().to_string();
                    if !self.state.playlist.contains(&s) {
                        self.state.playlist.push(s.clone());
                        self.state.playlist.sort_by_key(|p| song_sort_key(p));
                        self.rebuild_display_cache();
                        self.save_settings();
                        self.ingest_bands(&[s.clone()], None);
                    }
                    self.play_song(&s);
                    self.set_status(format!("Downloaded: {}", stem(&s)));
                }
                Msg::YtFail(e) => {
                    self.yt_pct = -1.0;
                    self.yt_speed.clear();
                    self.yt_eta.clear();
                    self.set_error(e);
                }
                Msg::Transcoded { display, wav } => {
                    let wavs = wav.to_string_lossy().to_string();
                    if let Some((pd, pa)) = self.pending_transcode.take() {
                        self.transcodes.insert(pa.clone(), wavs.clone());
                        if pd == display {
                            self.do_play(&display, &pa);
                        }
                    } else if !self.transcodes.contains_key(&display) {
                        self.transcodes.insert(display.clone(), wavs);
                    }
                }
                Msg::TranscodeFail { display, err } => {
                    if let Some((pd, _)) = self.pending_transcode.take() {
                        if pd == display {
                            self.state.current_song = None;
                            self.playing = String::new();
                            self.pos = 0.0;
                            self.len_secs = 0.0;
                            self.play_started = None;
                            self.set_error(format!("{}: {}", stem(&display), err));
                        }
                    }
                }
                Msg::Recorded { display, path } => {
                    self.recording = false;
                    let p = Path::new(&path);
                    let dir = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                    self.set_status(format!("Recorded: {} → {}", stem(&display), dir));
                }
                Msg::RecordFail { display, err } => {
                    self.recording = false;
                    self.set_error(format!("{}: {}", stem(&display), err));
                }
                Msg::VideoFrame { w, h, rgba } => {
                    self.video_dims = (w, h);
                    let size = [w as usize, h as usize];
                    let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    self.video_tex = Some(self.ctx.load_texture("video", img, egui::TextureOptions::LINEAR));
                    if self.video_audio_pending && !self.video_paused {
                        self.video_audio_pending = false;
                        if let Some(s) = &self.video_sink {
                            s.play();
                        }
                    }
                }
                Msg::VideoClosed { gen } => {
                    let was_eof = gen == self.video_gen && !self.video_closing;
                    if was_eof {
                        if let Some(path) = self.video_current.clone() {
                            self.video_positions.remove(&path);
                            self.save_settings();
                        }
                        if !self.video_queue.is_empty() {
                            self.queue_next();
                        } else {
                            self.video_ended = true;
                            self.video_paused = true;
                            self.set_status("MOVIE: end - PLAY to replay");
                        }
                    }
                }
                Msg::VideoPos { gen, secs } => {
                    if gen == self.video_gen {
                        self.video_cur_secs = secs;
                        if self.intro_skip_enabled && self.video_dur_secs > 0.0 {
                            let (ie, cs) = self.video_bound_secs();
                            if secs < ie && ie > 2.0 {
                                self.seek_video(ie + 1.0);
                                self.set_status(format!("Skipped intro → {:.0}s ({})", ie, fmt_time(ie)));
                            } else if cs > 2.0 && secs >= cs {
                                if !self.video_queue.is_empty() {
                                    self.queue_next();
                                    self.set_status("Skipped credits → next video");
                                }
                            }
                        }
                    }
                }
                Msg::VideoMeta { gen, dur } => {
                    if gen == self.video_gen {
                        self.video_dur_secs = dur;
                    }
                }
                Msg::VideoBounds { gen, path, intro_end, credits_start } => {
                    if gen == self.video_gen {
                        self.analyzing_video.remove(&path);
                        let had = self.video_bounds.get(&path).copied();
                        if (intro_end > 2.0 || credits_start > 2.0) && had != Some((intro_end, credits_start)) {
                            self.video_bounds.insert(path.clone(), (intro_end, credits_start));
                            if let Some(dir) = Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()) {
                                let merged = self.show_bounds.get(&dir).copied();
                                match merged {
                                    Some((pi, pc)) => {
                                        let ni = if intro_end > 2.0 { intro_end } else { pi };
                                        let nc = if credits_start > 2.0 { credits_start } else { pc };
                                        self.show_bounds.insert(dir, (ni, nc));
                                    }
                                    None => {
                                        self.show_bounds.insert(dir, (intro_end, credits_start));
                                    }
                                }
                            }
                            self.save_settings();
                        }
                    }
                }
            }
        }
    }

    fn load_art_texture(&mut self, bytes: Vec<u8>) {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let thumb = image::imageops::thumbnail(&rgba, 220, 220);
                let size = [thumb.width() as usize, thumb.height() as usize];
                let ci = ColorImage::from_rgba_unmultiplied(size, thumb.as_raw());
                self.art_tex = Some(self.ctx.load_texture("album_art", ci, TextureOptions::LINEAR));
                self.art_state = 2;
            }
            Err(_) => { self.art_state = 1; }
        }
    }

    fn play_bands(&self) -> [f32; 10] {
        self.eq_cur()
    }

    fn play_song(&mut self, path: &str) {
        if path.is_empty() || !Path::new(path).is_file() {
            self.set_error("File not found");
            return;
        }
        self.state.current_song = Some(path.to_string());
        self.eq_resume = None;
        self.state.prev_songs.push(path.to_string());
        self.state.song_count += 1;
        *self.state.play_history.entry(path.to_string()).or_insert(0) += 1;
        self.save_history();
        self.save_settings();
        self.playing = Self::playing_display(path);
        self.set_status(format!("Loading: {}...", stem(path)));

        let use_eq = self.eq_on && self.state.eq_preset != "Flat";
        if use_eq {
            self.eq_req_id += 1;
            let rid = self.eq_req_id;
            self.eq_dirty = false;
            self.eq_pending = Some(path.to_string());
            self.do_play(path, path);
            let bands = self.play_bands();
            self.set_status(format!("EQ processing: {}...", stem(path)));
            let _ = self.lib_tx.send(LibCmd::EqEncode(rid, path.to_string(), bands.to_vec()));
            self.send_meta_art(path.to_string());
            return;
        }
        self.do_play(path, path);
    }

    fn send_meta_art(&mut self, path: String) {
        self.meta = (stem(&path), "Unknown".to_string(), "Unknown".to_string());
        self.art_tex = None;
        self.art_state = 1;
        self.art_local_valid = false;
        let _ = self.lib_tx.send(LibCmd::Meta(path.clone()));
        let _ = self.lib_tx.send(LibCmd::ArtLocal(path));
    }

    fn do_play(&mut self, display: &str, audio: &str) {
        let src_path = if let Some(w) = self.transcodes.get(audio) {
            w.clone()
        } else {
            audio.to_string()
        };
        let src = match open_decoder(&src_path) {
            Some(s) => s,
            None => {
                if src_path != audio {
                    self.set_error(format!("Cached transcode unreadable: {}", stem(display)));
                    return;
                }
                if !ffmpeg_path().is_file() {
                    self.set_error(format!("Cannot decode {} (ffmpeg missing from tools folder)", stem(display)));
                    return;
                }
                self.begin_transcode(display, audio);
                return;
            }
        };
        self.play_decoded(display, src, true);
    }

    fn begin_transcode(&mut self, display: &str, audio: &str) {
        self.pending_transcode = Some((display.to_string(), audio.to_string()));
        self.set_status(format!("Converting {} with ffmpeg...", stem(display)));
        let _ = self.lib_tx.send(LibCmd::Transcode {
            display: display.to_string(),
            audio: audio.to_string(),
        });
    }

    fn play_decoded(&mut self, display: &str, src: Decoder<BufReader<File>>, forward_track: bool) {
        self.set_winding(None);
        if self.radio_on {
            self.stop_radio();
        }
        let new_track = forward_track && self.state.current_song.as_deref() != Some(display);
        if new_track && self.tape_out {
            self.tape_out = false;
        }
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.sink.append(src);
        self.sink.play();
        self.state.current_song = Some(display.to_string());
        self.state.prev_songs.push(display.to_string());
        self.state.song_count += 1;
        *self.state.play_history.entry(display.to_string()).or_insert(0) += 1;
        self.state.is_paused = false;
        if new_track {
            self.tape_songs += 1;
            if self.tape_songs > self.tape_cap {
                self.tape_songs = 1;
                self.tape_side = if self.tape_side == 'A' { 'B' } else { 'A' };
                self.tape_reversing = true;
                self.tape_timer = 0.0;
                self.set_status(format!("Auto-reverse  SIDE {}", self.tape_side));
            }
        }
        self.play_started = Some(Instant::now());
        let resume = self.eq_resume.take();
        if let Some(d) = resume {
            self.pos = if self.sink.try_seek(d).is_ok() { self.sink.get_pos().as_secs_f32() } else { 0.0 };
        } else {
            self.pos = 0.0;
        }
        self.dragging = false;
        self.save_history();
        self.save_settings();
        self.playing = Self::playing_display(display);
        self.set_status(format!("Playing: {}", stem(display)));
        self.send_meta_art(display.to_string());
    }

    fn swap_eq(&mut self, cur: &str) {
        let out = eq_out_path(self.eq_req_id);
        let src = match open_decoder(&out.to_string_lossy()) {
            Some(s) => s,
            None => {
                gc_eq_files(self.eq_req_id);
                self.set_error("EQ render unreadable, continuing original");
                return;
            }
        };
        gc_eq_files(self.eq_req_id);
        let pos = self.pos.max(0.0);
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.sink.append(src);
        self.sink.play();
        self.state.is_paused = false;
        self.play_started = Some(Instant::now());
        if self.sink.try_seek(Duration::from_secs_f32(pos)).is_ok() {
            self.pos = self.sink.get_pos().as_secs_f32();
        } else {
            self.pos = 0.0;
        }
        self.dragging = false;
        self.set_status(format!("EQ: {} applied -> {}", self.eq_name(), stem(cur)));
    }

    fn restart_current(&mut self) {
        let Some(cur) = self.state.current_song.clone() else { return };
        if self.len_secs > 0.0 && self.pos >= (self.len_secs - 0.5).max(0.0) {
            return;
        }
        if self.eq_pending.is_some() {
            self.eq_dirty = true;
            self.set_status("EQ: queued after current render");
            return;
        }
        self.eq_req_id += 1;
        let rid = self.eq_req_id;
        self.eq_dirty = false;
        let use_eq = self.eq_on && self.state.eq_preset != "Flat";
        if use_eq {
            self.eq_pending = Some(cur.clone());
            self.set_status(format!("Applying EQ: {}...", stem(&cur)));
            let bands = self.eq_cur();
            let _ = self.lib_tx.send(LibCmd::EqEncode(rid, cur.clone(), bands.to_vec()));
        } else {
            self.eq_pending = None;
            let pos = self.pos.max(0.0);
            self.do_play(&cur, &cur);
            if self.sink.try_seek(Duration::from_secs_f32(pos)).is_ok() {
                self.pos = self.sink.get_pos().as_secs_f32();
            }
            self.set_status(format!("EQ off: {}", stem(&cur)));
        }
    }

    fn skip_song(&mut self) {
        if let Some(cur) = self.state.current_song.clone() {
            if self.pos < 10.0 {
                let w = self.state.song_weights.entry(cur.clone()).or_insert(1.0);
                *w *= 0.8;
            }
        }
        if self.state.playlist.is_empty() || self.state.is_paused {
            return;
        }
        let pl = self.state.playlist.clone();
        let weights: HashMap<String, f32> = self.state.song_weights.clone();
        let (song, mut idx) = if self.disc_in {
            let i = self.state.current_song.as_ref().and_then(|c| pl.iter().position(|p| p == c)).unwrap_or(0);
            let ni = (i + 1) % pl.len();
            (pl[ni].clone(), Some(ni))
        } else if self.state.playlist_only_mode && self.state.playlist_only_sequential {
            let ni = self.state.song_count as usize % pl.len();
            (pl[ni].clone(), Some(ni))
        } else if !self.random_on {
            let i = self.state.current_song.as_ref().and_then(|c| pl.iter().position(|p| p == c)).unwrap_or(0);
            let ni = (i + 1) % pl.len();
            (pl[ni].clone(), Some(ni))
        } else if self.state.playlist_only_mode {
            (random_next(self.state.current_song.as_deref(), &pl, &weights), None)
        } else if self.state.dir_sequential {
            if let Some(dir) = self.state.current_dir.clone() {
                let files = collect_audio(&dir);
                if !files.is_empty() {
                    let picked = files[self.state.song_count as usize % files.len()].clone();
                    let pi = pl.iter().position(|p| *p == picked);
                    (picked, pi)
                } else {
                    (random_next(self.state.current_song.as_deref(), &pl, &weights), None)
                }
            } else {
                (random_next(self.state.current_song.as_deref(), &pl, &weights), None)
            }
        } else {
            (random_next(self.state.current_song.as_deref(), &pl, &weights), None)
        };
        if idx.is_none() {
            idx = pl.iter().position(|p| *p == song);
        }
        self.state.skip_count += 1;
        self.playing_pl_idx = idx;
        self.play_song(&song);
    }

    fn prev_song(&mut self) {
        if self.state.playlist.is_empty() || self.state.is_paused {
            return;
        }
        let pl = self.state.playlist.clone();
        let i = self.state.current_song.as_ref().and_then(|c| pl.iter().position(|p| p == c)).unwrap_or(0);
        let pi = (i + pl.len() - 1) % pl.len();
        let prev = pl[pi].clone();
        self.playing_pl_idx = Some(pi);
        self.play_song(&prev);
    }

    fn toggle_pause(&mut self) {
        if self.sink.empty() && self.state.current_song.is_none() {
            return;
        }
        if self.state.is_paused {
            self.state.is_paused = false;
            self.sink.play();
            self.set_status("Playing");
        } else {
            self.state.is_paused = true;
            self.sink.pause();
            self.set_status("Paused");
        }
    }

    fn pause_music(&mut self) {
        self.state.is_paused = true;
        self.sink.pause();
        self.set_status("Paused");
    }

    fn unpause_music(&mut self) {
        self.state.is_paused = false;
        self.sink.play();
        self.set_status("Playing");
    }

    fn stop_music(&mut self) {
        self.sink.stop();
        self.sink.set_speed(1.0);
        self.play_started = None;
        self.winding = None;
        self.wind_arm = None;
        self.wind_playing = false;
    }

    fn resume_tape(&mut self, path: &str, pos: f32) {
        let pos = pos.max(0.0);
        self.eq_resume = Some(Duration::from_secs_f32(pos));
        self.state.is_paused = false;
        self.state.current_song = Some(path.to_string());
        self.playing_pl_idx = self.state.playlist.iter().position(|p| p == path);
        self.playing = Self::playing_display(path);
        self.do_play(path, path);
        self.set_status(format!("TAPE resumed: {}", stem(path)));
    }

    fn set_deck_mode(&mut self, m: DeckMode) {
        if self.deck_mode == m {
            if m != DeckMode::Tape {
                return;
            }
            if self.play_started.is_none() && !self.radio_on {
                self.stop_radio();
                if let Some((path, pos)) = self.deck_resume.take() {
                    if Path::new(&path).is_file() {
                        self.resume_tape(&path, pos);
                        return;
                    }
                }
                if let Some(cur) = self.state.current_song.clone() {
                    if Path::new(&cur).is_file() {
                        self.resume_tape(&cur, 0.0);
                    }
                }
            }
            return;
        }
        if m == DeckMode::Tape {
            let from_disc = self.deck_mode == DeckMode::Disc;
            self.stop_radio();
            self.deck_mode = m;
            if from_disc {
                self.disc_in = false;
                self.disc_label = String::new();
                self.state.playlist = self.all_songs_ordered();
                self.playing_pl_idx = None;
                self.rebuild_display_cache();
                self.save_settings();
            }
            if let Some((path, pos)) = self.deck_resume.take() {
                if Path::new(&path).is_file() {
                    self.resume_tape(&path, pos);
                }
            }
            return;
        }
        if self.deck_mode == DeckMode::Tape {
            if let Some(cur) = self.state.current_song.clone() {
                if Path::new(&cur).is_file() {
                    self.deck_resume = Some((cur, self.pos.max(0.0)));
                }
            }
        }
        self.stop_radio();
        self.deck_mode = m;
    }

    fn radio_band_range(&self) -> (f32, f32) {
        if self.radio_band == "AM" {
            (530.0, 1700.0)
        } else {
            (87.5, 108.0)
        }
    }

    fn radio_tuned_at(&self) -> Option<usize> {
        let tol = if self.radio_band == "AM" { 25.0 } else { 0.3 };
        let mut best: Option<(usize, f32)> = None;
        for (i, s) in self.radio_presets.iter().enumerate() {
            if !s.band.eq_ignore_ascii_case(&self.radio_band) {
                continue;
            }
            let d = (s.freq - self.radio_freq).abs();
            if d <= tol && best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    fn start_radio(&mut self, idx: usize) {
        if idx >= self.radio_presets.len() {
            return;
        }
        if self.radio_last_start.elapsed() < Duration::from_millis(1200) && self.radio_on {
            return;
        }
        let st = self.radio_presets[idx].clone();
        self.set_winding(None);
        if let Some(mut c) = self.radio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.radio_loading = true;
        match spawn_radio_stream(&st.url, &ffmpeg_path()) {
            Ok((proc, src)) => {
                self.sink.append(src);
                self.sink.play();
                self.state.is_paused = false;
                self.radio_proc = Some(proc);
                self.radio_playing_idx = Some(idx);
                self.radio_on = true;
                self.radio_loading = false;
                self.radio_last_start = Instant::now();
                self.play_started = None;
                self.state.current_song = None;
                self.set_status(format!("♪ {} · {}", fmt_freq(st.freq), st.name));
            }
            Err(e) => {
                self.radio_loading = false;
                self.radio_on = false;
                self.radio_playing_idx = None;
                self.set_error(format!("Radio: {}", e));
            }
        }
    }

    fn stop_radio(&mut self) {
        if let Some(mut c) = self.radio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.radio_proc = None;
        self.radio_on = false;
        self.radio_playing_idx = None;
        self.radio_loading = false;
        self.sink.stop();
        self.play_started = None;
        self.state.is_paused = false;
    }

    fn tune_radio_to(&mut self, freq: f32, now: f64) {
        let (lo, hi) = self.radio_band_range();
        self.radio_freq = freq.clamp(lo, hi);
        self.radio_last_moved = now;
    }

    fn tick_radio(&mut self, now: f64) {
        if !self.show_radio && !self.radio_on {
            return;
        }
        let target = self.radio_tuned_at();
        self.radio_tuned = target;
        if now - self.radio_last_moved > 0.7 {
            if let Some(t) = target {
                if self.radio_playing_idx != Some(t) {
                    self.start_radio(t);
                }
            }
        }
        if self.radio_on && !self.radio_loading {
            if self.sink.empty() && self.play_started.is_none() {
                if let Some(idx) = self.radio_playing_idx {
                    if self.radio_last_start.elapsed() > Duration::from_millis(2000) {
                        self.radio_proc = None;
                        self.start_radio(idx);
                    }
                }
            }
        }
    }

    fn scan_radio(&mut self, now: f64) {
        let (lo, hi) = self.radio_band_range();
        let mut cands: Vec<usize> = self
            .radio_presets
            .iter()
            .enumerate()
            .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band) && s.freq >= lo && s.freq <= hi)
            .map(|(i, _)| i)
            .collect();
        cands.sort_by(|&a, &b| {
            let fa = self.radio_presets[a].freq;
            let fb = self.radio_presets[b].freq;
            let ka = if fa <= self.radio_freq { 1 } else { 0 };
            let kb = if fb <= self.radio_freq { 1 } else { 0 };
            ka.cmp(&kb).then_with(|| fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal))
        });
        if let Some(&i) = cands.first() {
            self.tune_radio_to(self.radio_presets[i].freq, now);
            self.start_radio(i);
        }
    }

    fn record_radio(&mut self) {
        if self.recording {
            return;
        }
        let Some(idx) = self.radio_playing_idx else {
            self.set_error("Tune a station first");
            return;
        };
        let st = self.radio_presets[idx].clone();
        let rec_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Recordings");
        let safe: String = st
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let mut dest = rec_dir.join(format!("{}_RADIO.wav", safe));
        let mut n = 2;
        while dest.exists() {
            dest = rec_dir.join(format!("{}_RADIO ({}).wav", safe, n));
            n += 1;
        }
        let _ = self.lib_tx.send(LibCmd::RecordRadio {
            display: format!("{} {}", st.band, st.name),
            url: st.url,
            dest: dest.to_string_lossy().to_string(),
        });
        self.recording = true;
        self.set_status(format!("Recording 30s of {}...", st.name));
    }

    fn insert_disc(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(d) = self.last_cd_dir.clone() {
            if Path::new(&d).is_dir() {
                dialog = dialog.set_directory(d);
            }
        }
        if let Some(dir) = dialog.pick_folder() {
            let files = collect_audio(&dir.to_string_lossy());
            if files.is_empty() {
                self.set_error("No audio in that folder");
                return;
            }
            self.last_cd_dir = Some(dir.to_string_lossy().to_string());
            self.disc_saved = self.state.playlist.clone();
            self.state.playlist = files;
            self.rebuild_display_cache();
            self.disc_in = true;
            self.disc_label = dir
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| dir.to_string_lossy().to_string());
            self.save_settings();
            self.ingest_bands(&self.state.playlist.clone(), Some(self.disc_label.clone()));
            self.set_status(format!("DISC IN: {}", self.disc_label));
            if !self.state.playlist.is_empty() {
                let first = self.state.playlist[0].clone();
                if self.state.is_paused { self.unpause_music(); }
                self.play_song(&first);
            }
        }
    }

    fn eject_disc(&mut self) {
        if !self.disc_in {
            return;
        }
        self.disc_in = false;
        if !self.disc_saved.is_empty() {
            self.state.playlist = self.disc_saved.clone();
        }
        self.state.current_song = None;
        self.playing_pl_idx = None;
        self.stop_music();
        if self.radio_on {
            self.stop_radio();
        }
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("DISC EJECTED");
    }

    fn ingest_bands(&mut self, files: &[String], group: Option<String>) {
        let mut changed = false;
        for f in files {
            let band = group.clone().unwrap_or_else(|| band_of(f));
            if band.is_empty() { continue; }
            if !self.full_library.contains(f) {
                self.full_library.push(f.clone());
                changed = true;
            }
            let list = self.band_map.entry(band.clone()).or_default();
            if !list.contains(f) {
                list.push(f.clone());
                changed = true;
            }
        }
        if changed {
            self.sort_bands_library();
            self.save_bands();
        }
    }

    fn sort_bands_library(&mut self) {
        self.full_library.sort_by_key(|p| song_sort_key(p));
        for list in self.band_map.values_mut() {
            list.sort_by_key(|p| song_sort_key(p));
        }
    }

    fn ingest_dir(&mut self, dir: &str, files: &[String]) {
        let root = Path::new(dir);
        let root_name = root.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let nested_cnt = files.iter().filter(|f| Path::new(f).parent() != Some(root)).count();
        let library_mode = nested_cnt > 0 && nested_cnt > files.len() - nested_cnt;
        let mut changed = false;
        for f in files {
            let p = Path::new(f);
            let band = if library_mode {
                if p.parent() == Some(root) {
                    band_of_file(f)
                } else {
                    p.strip_prefix(root)
                        .ok()
                        .and_then(|rel| rel.components().next())
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .unwrap_or_else(|| root_name.clone())
                }
            } else {
                root_name.clone()
            };
            if band.is_empty() { continue; }
            if !self.full_library.contains(f) {
                self.full_library.push(f.clone());
                changed = true;
            }
            let list = self.band_map.entry(band.clone()).or_default();
            if !list.contains(f) {
                list.push(f.clone());
                changed = true;
            }
        }
        if changed {
            self.sort_bands_library();
            self.save_bands();
        }
    }

    fn save_bands(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&self.band_map) {
            let _ = std::fs::write(&self.bands_path, text);
        }
    }

    fn draw_play_switch(&mut self, ui: &mut egui::Ui, th: &ThemePalette) {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("PLAY ORDER").color(darken(th.accent, 0.35)).monospace().size(8.0));
        });
        let size = egui::vec2(64.0, 118.0);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        painter.rect_filled(rect, 9.0, lighten(th.btn_bg, 0.12));
        painter.rect_stroke(rect, 9.0, egui::Stroke::new(2.0, th.btn_fg), egui::StrokeKind::Outside);
        let corners = [
            rect.min + egui::vec2(6.0, 6.0),
            rect.min + egui::vec2(rect.width() - 6.0, 6.0),
            rect.min + egui::vec2(6.0, rect.height() - 6.0),
            rect.max - egui::vec2(6.0, 6.0),
        ];
        for s in corners {
            painter.circle_filled(s, 2.6, darken(th.btn_bg, 0.5));
            if darken(th.btn_fg, 0.3) != Color32::TRANSPARENT {
                painter.circle_stroke(s, 2.6, egui::Stroke::new(1.0, darken(th.btn_fg, 0.3)));
            }
        }
        let groove = rect.shrink2(egui::vec2(19.0, 18.0));
        painter.rect_filled(groove, 5.0, Color32::from_rgb(2, 2, 5));
        painter.rect_stroke(groove, 5.0, egui::Stroke::new(1.5, darken(th.btn_fg, 0.25)), egui::StrokeKind::Inside);
        painter.text(
            egui::pos2(groove.center().x, groove.min.y + 10.0),
            egui::Align2::CENTER_CENTER,
            "ON",
            egui::FontId::monospace(8.0),
            if self.random_on { th.playing_fg } else { Color32::from_gray(60) },
        );
        painter.text(
            egui::pos2(groove.center().x, groove.max.y - 10.0),
            egui::Align2::CENTER_CENTER,
            "OFF",
            egui::FontId::monospace(8.0),
            if !self.random_on { th.playing_fg } else { Color32::from_gray(60) },
        );
        let thr = 26.0;
        if resp.dragged() {
            self.switch_accum = (self.switch_accum - resp.drag_delta().y).clamp(-thr, thr);
        } else if resp.clicked() {
            self.random_on = !self.random_on;
            self.switch_accum = 0.0;
        }
        if resp.drag_stopped() {
            if self.switch_accum.abs() >= thr * 0.5 {
                self.random_on = self.switch_accum > 0.0;
            }
            self.switch_accum = 0.0;
            if self.random_on {
                self.set_status("Random shuffle ON — skip picks a random track");
            } else {
                self.set_status("Normal order ON — tracks play in playlist order");
            }
        }
        let base = if self.random_on { 1.0 } else { 0.0 };
        let kf = (base + self.switch_accum / thr).clamp(0.0, 1.0);
        let knw = 42.0;
        let travel = groove.height() - knw - 8.0;
        let kx = groove.min.x + 4.0;
        let ky = groove.min.y + 4.0 + (1.0 - kf) * (travel.max(0.0));
        let knob = egui::Rect::from_min_size(egui::pos2(kx, ky), egui::vec2(groove.width() - 8.0, knw));
        let knob_col = if self.random_on { mix(th.btn_bg, th.accent, 0.4) } else { darken(th.btn_bg, 0.2) };
        painter.rect_filled(knob, 6.0, knob_col);
        painter.rect_stroke(
            knob,
            6.0,
            egui::Stroke::new(1.5, if self.random_on { lighten(th.accent, 0.15) } else { th.btn_fg }),
            egui::StrokeKind::Inside,
        );
        painter.hline(knob.min.x + 6.0..=knob.max.x - 6.0, knob.center().y, egui::Stroke::new(2.0, darken(knob_col, 0.35)));
        ui.add_space(2.0);
        ui.vertical_centered(|ui| {
            let (txt, col) = if self.random_on { ("RANDOM SHUFFLE", th.accent) } else { ("NORMAL ORDER", th.playing_fg) };
            ui.label(RichText::new(txt).color(col).monospace().size(9.0));
        });
    }

    fn all_songs_ordered(&self) -> Vec<String> {
        let mut band_names: Vec<String> = self.band_map.keys().cloned().collect();
        band_names.sort_by_key(|b| b.to_lowercase());
        let mut seen: HashSet<String> = HashSet::new();
        let mut ordered: Vec<(String, String)> = Vec::new();
        for b in band_names {
            if let Some(v) = self.band_map.get(&b) {
                let mut songs = v.clone();
                songs.sort_by_key(|p| song_sort_key(p));
                for s in songs {
                    if seen.insert(s.clone()) {
                        ordered.push((b.clone(), s));
                    }
                }
            }
        }
        for f in self.full_library.iter() {
            if seen.insert(f.clone()) {
                ordered.push((band_of(f), f.clone()));
            }
        }
        ordered.sort_by_key(|(b, _)| b.to_lowercase());
        ordered.into_iter().map(|(_, s)| s).collect()
    }

    fn choose_band(&mut self, band: String) {
        self.band_sel = band.clone();
        self.disc_in = false;
        self.state.playlist = if band.is_empty() {
            self.all_songs_ordered()
        } else if let Some(v) = self.band_map.get(&band) {
            let mut songs = v.clone();
            songs.sort_by_key(|p| song_sort_key(p));
            songs
        } else {
            Vec::new()
        };
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status(
            if band.is_empty() {
                format!("All tracks: {}", self.state.playlist.len())
            } else {
                format!("Band {}: {} songs", band, self.state.playlist.len())
            },
        );
        if self.radio_on {
            self.stop_radio();
        }
        if !self.state.playlist.is_empty() && !self.state.is_paused {
            self.skip_song();
        }
    }

    fn clear_bands(&mut self) {
        self.band_map.clear();
        self.full_library.clear();
        self.band_sel = String::new();
        self.save_bands();
        self.set_status("Band library cleared");
    }

    fn set_volume(&mut self, level: u32) {
        self.state.volume = level.clamp(0, 100);
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.save_settings();
    }

    fn increase_volume(&mut self) {
        self.set_volume(self.state.volume + 5);
    }

    fn decrease_volume(&mut self) {
        self.set_volume(self.state.volume.saturating_sub(5));
    }

    fn seek_to(&self, secs: f32) {
        if self.state.current_song.is_none() || self.state.is_paused {
            return;
        }
        let _ = self.sink.try_seek(Duration::from_secs_f32(secs.max(0.0)));
    }

    fn seek_skip(&mut self, d: f32) {
        if self.state.current_song.is_none() {
            return;
        }
        let np = (self.pos + d).clamp(0.0, self.len_secs);
        self.pos = np;
        if !self.state.is_paused {
            let _ = self.sink.try_seek(Duration::from_secs_f32(np));
        }
    }

    fn skip_forward(&mut self) {
        self.seek_skip(10.0);
    }

    fn skip_backward(&mut self) {
        self.seek_skip(-10.0);
    }

    fn step_tape(&mut self, dt: f32) {
        if self.tape_reversing {
            const OUT: f32 = 0.55;
            const HOLD: f32 = 0.5;
            self.tape_timer += dt;
            if self.tape_timer < OUT {
                self.tape_anim = self.tape_timer / OUT;
            } else if self.tape_timer < OUT + HOLD {
                self.tape_anim = 1.0;
            } else if self.tape_timer < OUT * 2.0 + HOLD {
                self.tape_anim = 1.0 - (self.tape_timer - OUT - HOLD) / OUT;
            } else {
                self.tape_reversing = false;
                self.tape_anim = 0.0;
            }
        } else if self.tape_out {
            self.tape_anim = (self.tape_anim + dt / 0.5).min(1.0);
        } else {
            self.tape_anim = (self.tape_anim - dt / 0.4).max(0.0);
        }
    }

    fn set_winding(&mut self, w: Option<f32>) {
        let prev = self.winding;
        self.winding = w;
        self.wind_arm = None;
        if self.state.current_song.is_none() {
            return;
        }
        if prev == w {
            return;
        }
        // Disengage previous
        if prev == Some(1.0) {
            self.sink.set_speed(1.0);
        } else if prev == Some(-1.0) && self.wind_playing {
            self.sink.play();
            let _ = self.sink.try_seek(Duration::from_secs_f32(self.pos));
        }
        // Engage new
        match w {
            Some(1.0) => {
                if !self.state.is_paused && self.play_started.is_some() {
                    self.sink.set_speed(6.0);
                }
            }
            Some(-1.0) => {
                self.wind_playing = !self.state.is_paused && self.play_started.is_some();
                if self.wind_playing {
                    self.sink.pause();
                }
            }
            _ => {}
        }
    }

    fn step_wind(&mut self, dt: f32) {
        let Some(dir) = self.winding else { return };
        if self.state.current_song.is_none() {
            return;
        }
        let speed = (self.len_secs / 25.0).max(3.0);
        if dir > 0.0 && !self.state.is_paused && self.play_started.is_some() {
            let cur = self.sink.get_pos().as_secs_f32();
            self.pos = cur.clamp(0.0, self.len_secs);
            if self.pos >= self.len_secs && self.len_secs > 0.0 {
                self.set_winding(None);
            }
        } else {
            let np = (self.pos + dir * speed * dt).clamp(0.0, self.len_secs);
            self.pos = np;
        }
    }

    fn cycle_theme(&mut self) {
        let idx = THEME_NAMES.iter().position(|t| *t == self.state.theme).unwrap_or(0);
        self.state.theme = THEME_NAMES[(idx + 1) % THEME_NAMES.len()].to_string();
        self.save_settings();
        self.set_status(format!("Theme: {}", self.state.theme));
    }

    #[allow(dead_code)]
    fn cycle_eq(&mut self) {
        self.eq_locked_notice();
    }

    #[allow(dead_code)]
    fn cycle_eq_back(&mut self) {
        self.eq_locked_notice();
    }

    #[allow(dead_code)]
    fn eq_set(&mut self, _i: usize, _v: f32) {
        self.eq_locked_notice();
    }

    fn eq_locked_notice(&mut self) {
        self.eq_on = false;
        self.eq_custom = None;
        if self.state.eq_preset != "Flat" {
            self.state.eq_preset = "Flat".to_string();
        }
        self.set_status("EQ disabled: caused issues, forced to Flat");
    }

    fn toggle_repeat(&mut self) {
        self.state.repeat_enabled = !self.state.repeat_enabled;
        self.set_status(format!("Repeat {}", if self.state.repeat_enabled { "On" } else { "Off" }));
    }

    fn toggle_playlist_only(&mut self) {
        self.state.playlist_only_mode = !self.state.playlist_only_mode;
        self.set_status(format!("Playlist Only: {}", if self.state.playlist_only_mode { "On" } else { "Off" }));
    }

    fn toggle_playlist_sequential(&mut self) {
        self.state.playlist_only_sequential = !self.state.playlist_only_sequential;
        self.set_status(format!("Sequential: {}", if self.state.playlist_only_sequential { "On" } else { "Off" }));
    }

    fn toggle_dir_sequential(&mut self) {
        self.state.dir_sequential = !self.state.dir_sequential;
        self.set_status(format!("Dir Seq: {}", if self.state.dir_sequential { "On" } else { "Off" }));
    }

    fn smart_shuffle(&mut self) {
        if self.state.playlist.len() < 3 {
            self.set_error("Need more songs in playlist");
            return;
        }
        let weight: f32 = self.state.playlist.iter().map(|s| self.state.song_weights.get(s).copied().unwrap_or(1.0)).sum();
        if weight <= 0.0 {
            self.set_error("No weighted songs yet, play more!");
            return;
        }
        self.set_status(format!("Smart shuffle: {} songs", self.state.playlist.len()));
    }

    fn req_record(&mut self) {
        if self.recording {
            return;
        }
        let Some(s) = self.state.current_song.clone() else { return };
        if !Path::new(&s).is_file() {
            self.set_error("Nothing to record");
            return;
        }
        let base = stem(&s);
        let rec_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Recordings");
        let mut dest = rec_dir.join(format!("{}_REC.wav", base));
        let mut n = 2;
        while dest.exists() {
            dest = rec_dir.join(format!("{}_REC ({}).wav", base, n));
            n += 1;
        }
        let display = self
            .state
            .playlist
            .iter()
            .position(|p| *p == s)
            .and_then(|i| self.display_cache.get(i).cloned())
            .unwrap_or_else(|| s.clone());
        let _ = self.lib_tx.send(LibCmd::Record {
            display,
            audio: s,
            dest: dest.to_string_lossy().to_string(),
        });
        self.recording = true;
        self.set_status("Recording...");
    }

    fn save_song(&mut self) {
        if let Some(s) = self.state.current_song.clone() {
            if !self.state.playlist.contains(&s) {
                self.state.playlist.push(s);
                self.rebuild_display_cache();
                self.save_settings();
                self.set_status("Song saved to playlist");
            }
        }
    }

    fn clear_playlist(&mut self) {
        self.state.playlist.clear();
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("Playlist cleared");
    }

    fn shuffle_playlist(&mut self) {
        let mut rng = rand::rng();
        let mut pl = std::mem::take(&mut self.state.playlist);
        let n = pl.len();
        for i in (1..n).rev() {
            let j = rng.random_range(0..=i);
            pl.swap(i, j);
        }
        self.state.playlist = pl;
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("Playlist shuffled");
    }

    fn move_playlist(&mut self, from: usize, to: usize) {
        if from >= self.state.playlist.len() || to > self.state.playlist.len() || from == to {
            return;
        }
        let mut pl = std::mem::take(&mut self.state.playlist);
        let s = pl.remove(from);
        pl.insert(to, s);
        self.state.playlist = pl;
        self.rebuild_display_cache();
        self.save_settings();
    }

    fn set_sleep_timer(&mut self, minutes: i64) {
        if minutes > 0 {
            self.sleep_minutes = minutes as u32;
            self.sleep_deadline = Some(Instant::now() + Duration::from_secs(minutes as u64 * 60));
            self.set_status(format!("Sleep timer: {} min", minutes));
        } else {
            self.sleep_minutes = 0;
            self.sleep_deadline = None;
            self.set_status("Sleep timer off");
        }
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        self.save_settings();
        self.save_history();
        let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn update_playback_state(&mut self) {
        if !self.state.is_paused {
            if let Some(start) = self.play_started {
                if start.elapsed() > Duration::from_millis(600) && self.sink.empty() && self.state.current_song.is_some() {
                    let now = Instant::now();
                    if now.duration_since(self.last_skip) > Duration::from_millis(1500) {
                        self.last_skip = now;
                        if self.state.repeat_enabled {
                            if let Some(s) = self.state.current_song.clone() {
                                self.play_song(&s);
                            }
                        } else {
                            self.skip_song();
                        }
                    }
                }
            }
        }
        if !self.state.is_paused && self.state.current_song.is_some() {
            if self.winding.is_none() && !self.dragging {
                self.pos = self.sink.get_pos().as_secs_f32();
            }
        }
        if self.state.start_time.is_some() {
            let d = self.state.start_time.as_ref().map(|t| t.elapsed()).unwrap_or_default();
            self.running = format!("Songs: {} | Skips: {} | {}", self.state.song_count, self.state.skip_count, fmt_run(d));
        }
        if let Some(until) = self.status_until {
            if Instant::now() > until {
                self.status = format!("{} v{}", APP_NAME, APP_VERSION);
                self.status_until = None;
            }
        }
        if let Some(until) = self.error_until {
            if Instant::now() > until {
                self.error.clear();
                self.error_until = None;
            }
        }
    }

    fn handle_drops(&mut self) {
        let dropped: Vec<PathBuf> = self
            .ctx
            .input(|i| i.raw.dropped_files.iter().map(|d| d.path().to_path_buf()).collect());
        if !dropped.is_empty() && dropped != self.last_drop {
            self.last_drop = dropped.clone();
            let entries: Vec<String> = dropped.iter().map(|p| p.to_string_lossy().to_string()).collect();
            self.set_status("Adding files...");
            let _ = self.lib_tx.send(LibCmd::AddMany(entries));
        }
    }

    fn poll_ext(&mut self) {
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if e.id == MenuId::new("show") {
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if e.id == MenuId::new("play_pause") {
                self.toggle_pause();
            } else if e.id == MenuId::new("skip") {
                self.skip_song();
            } else if e.id == MenuId::new("prev") {
                self.prev_song();
            } else if e.id == MenuId::new("stop") {
                self.stop_music();
            } else if e.id == MenuId::new("quit") {
                let ctx = self.ctx.clone();
                self.request_quit(&ctx);
            }
        }
        if let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state == HotKeyState::Pressed {
                if e.id == self.show_id {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else if e.id == self.quit_id {
                    let ctx = self.ctx.clone();
                    self.request_quit(&ctx);
                }
            }
        }
    }

    fn handle_keys(&mut self) {
        let focused = self.ctx.memory(|m| m.focused().is_some());
        let (space, left, right, up, down, f, f10, esc, shift, alt_enter) = self.ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::F10),
                i.key_pressed(egui::Key::Escape),
                i.modifiers.shift,
                i.modifiers.alt && i.key_pressed(egui::Key::Enter),
            )
        });
        if !focused {
            if self.video_on {
                if space { self.toggle_video_play(); }
                if left { self.seek_video((self.video_cur_secs - 5.0).max(0.0)); }
                if right { self.seek_video((self.video_cur_secs + 5.0).min((self.video_dur_secs - 1.0).max(0.0))); }
            } else {
                if space { self.toggle_pause(); }
                if right {
                    if shift { self.skip_forward(); } else { self.skip_song(); }
                }
                if left {
                    if shift { self.skip_backward(); } else { self.prev_song(); }
                }
                if up { self.increase_volume(); }
                if down { self.decrease_volume(); }
            }
        }
        if f { self.toggle_fullscreen(); }
        if alt_enter {
            if self.video_on {
                self.set_video_fs(!self.video_fs);
            } else {
                self.toggle_fullscreen();
            }
        }
        if esc && self.fullscreen { self.set_fullscreen(false); }
        if esc && self.video_fs { self.set_video_fs(false); }
        if f10 {
            let ctx = self.ctx.clone();
            self.request_quit(&ctx);
        }

        let chars: Vec<char> = self.ctx.input(|i| {
            i.events.iter().filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.chars().collect::<Vec<char>>()),
                _ => None,
            }).flatten().collect()
        });
        if !focused && !self.video_on {
            for c in chars {
                match c {
                    '+' | '=' => self.increase_volume(),
                    '-' | '_' => self.decrease_volume(),
                    's' | 'S' => self.smart_shuffle(),
                    't' | 'T' => self.cycle_theme(),
                    'z' | 'Z' => self.prev_song(),
                    'x' | 'X' => self.toggle_pause(),
                    'c' | 'C' => self.stop_music(),
                    'v' | 'V' => self.toggle_playlist_only(),
                    'b' | 'B' => self.toggle_repeat(),
                    'j' | 'J' => self.search_focus = true,
                    _ => {}
                }
            }
        }
    }

    fn toggle_fullscreen(&mut self) {
        let next = !self.fullscreen;
        self.set_fullscreen(next);
    }

    fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        if on {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        } else {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    fn set_video_fs(&mut self, on: bool) {
        if on && !self.video_fs {
            if !self.fullscreen {
                self.video_fs_restore_app = true;
                self.set_fullscreen(true);
            } else {
                self.video_fs_restore_app = false;
            }
        }
        if !on && self.video_fs && self.video_fs_restore_app {
            self.video_fs_restore_app = false;
            self.set_fullscreen(false);
        }
        self.video_fs = on;
        if !on {
            self.video_pos = egui::pos2(60.0, 60.0);
        } else {
            self.video_last_mouse = 0.0;
        }
    }

    fn draw_sleep_timer(&mut self, ctx: &egui::Context) {
        if self.fading {
            let t = (Instant::now() - self.fade_start).as_secs_f32();
            let frac = (1.0 - t / 3.0).clamp(0.0, 1.0);
            self.sink.set_volume(self.fade_from_vol as f32 / 100.0 * frac);
            if t >= 4.0 {
                self.request_quit(ctx);
            }
            return;
        }
        if let Some(d) = self.sleep_deadline {
            if Instant::now() >= d {
                let _ = self.sink.try_seek(Duration::ZERO);
                self.fade_start = Instant::now();
                self.fade_from_vol = self.state.volume;
                self.fading = true;
                self.sleep_deadline = None;
            }
        }
    }

    fn marquee_text(&self) -> String {
        if let Some(s) = self.state.current_song.clone() {
            if self.meta.1 != "Unknown" {
                format!("♪ {} - {}", self.meta.1, self.meta.0)
            } else {
                format!("♪ {}", stem(&s))
            }
        } else {
            self.status.clone()
        }
    }

    fn update_viz(&mut self, dt: f32) {
        let t = self.ctx.input(|i| i.time) as f32;
        let vh = 76.0;
        for i in 0..self.viz_bars.len() {
            let x = i as f32;
            let mut h = 0.0;
            h += (t * 1.7 + x * 0.83).sin().abs() * 0.30;
            h += (t * 3.4 + x * 1.37).sin().abs() * 0.24;
            h += (t * 5.9 + x * 2.21).sin().abs() * 0.18;
            let bass = if x < 4.0 { (t * 2.05 + x * 0.3).sin().abs() * 0.22 } else { 0.0 };
            let mut target = (h + bass + 0.08) * vh * 1.25;
            if target < 3.0 {
                target = 3.0;
            }
            let cur = self.viz_bars[i];
            let k = if target > cur { 1.0 - (-dt * 16.0).exp() } else { 1.0 - (-dt * 5.5).exp() };
            let next = cur + (target - cur) * k;
            self.viz_bars[i] = next.max(2.0).min(vh);
            let peak = self.viz_peaks[i];
            if next > peak {
                self.viz_peaks[i] = next;
            } else {
                self.viz_peaks[i] = (peak - dt * 9.0).max(0.0);
            }
        }
    }

    fn draw_title_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::top("title_panel").show(ui, |ui| {
            let rect = ui.max_rect();
            paint_title_gradient(&ui.painter().with_clip_rect(rect), rect, th.accent, th.bg);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(APP_NAME).color(Color32::from_rgb(255, 240, 160)).monospace().strong().size(14.0));
                ui.separator();
                let w = (ui.available_width() - 12.0).max(160.0);
                let (brect, _) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::hover());
                let painter = ui.painter().with_clip_rect(brect);
                painter.rect_filled(brect, 2.0, darken(th.bg, 0.1));
                painter.rect_stroke(brect, 2.0, egui::Stroke::new(1.0, darken(th.accent, 0.6)), egui::StrokeKind::Inside);
                let marquee = if self.state.is_paused {
                    format!("II  {}", self.marquee_text())
                } else {
                    self.marquee_text()
                };
                if !marquee.is_empty() {
                    let cx = 8.4;
                    let tw = marquee.chars().count() as f32 * cx;
                    let t = ui.ctx().input(|i| i.time) as f32;
                    let total = brect.width() + tw + 60.0;
                    let mut x = brect.min.x + brect.width() - (t * 40.0).rem_euclid(total);
                    if x + tw < brect.min.x {
                        x += total;
                    }
                    painter.text(
                        egui::pos2(x, brect.center().y),
                        egui::Align2::LEFT_CENTER,
                        marquee,
                        egui::FontId::monospace(13.0),
                        th.playing_fg,
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.running).color(th.fg).monospace());
            });
            if !self.error.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&self.error).color(Color32::from_rgb(255, 80, 80)).monospace().strong());
                });
            }
            ui.add_space(1.0);
        });
    }

    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::bottom("status_panel").show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().with_clip_rect(rect).rect_filled(
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + 1.0)),
                0.0,
                th.accent,
            );
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.status).color(th.fg).monospace().size(11.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if retro_btn(ui, "Exit", Color32::from_rgb(255, 110, 110), th.btn_bg).clicked() {
                        let c = self.ctx.clone();
                        self.request_quit(&c);
                    }
                    ui.label(RichText::new("Ctrl+Alt+M=Show | Ctrl+Alt+Q=Quit").color(Color32::from_gray(110)).monospace().size(9.0));
                });
            });
            ui.add_space(1.0);
        });
    }

    fn draw_side_panel(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::left("side_panel").show(ui, |ui| {
            ui.set_max_width(250.0);
            let (w, h) = (210.0, 200.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            let p = ui.painter().with_clip_rect(rect);
            engrave(&p, rect, th.btn_bg);
            p.rect_filled(rect.shrink(2.0), 2.0, th.art_bg);
            if let Some(tex) = &self.art_tex {
                p.image(
                    tex.id(),
                    egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(w - 8.0, h - 8.0)),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                p.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    match self.art_state { 1 => "Loading art...", _ => "No Art" },
                    egui::FontId::monospace(13.0),
                    Color32::from_gray(90),
                );
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&self.meta.0).color(th.fg).monospace().strong().size(13.0));
            ui.label(RichText::new(&self.meta.1).color(th.accent).monospace().size(11.0));
            ui.label(RichText::new(&self.meta.2).color(Color32::from_gray(140)).monospace().size(10.0));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("NOW").color(th.playing_fg).monospace().size(9.0));
                ui.label(RichText::new(&self.playing).color(th.playing_fg).monospace().size(12.0));
            });

            let dt = ui.input(|i| i.stable_dt);
            self.update_viz(dt);
            let (vrect, _) = ui.allocate_exact_size(egui::vec2(200.0, 96.0), egui::Sense::hover());
            let painter = ui.painter().with_clip_rect(vrect);
            engrave(&painter, vrect, th.btn_bg);
            painter.rect_filled(vrect.shrink(2.0), 2.0, th.art_bg);
            let vr = vrect.shrink(10.0);
            engrave(&painter, egui::Rect::from_min_max(vr.min - egui::vec2(2.0, 2.0), vr.max + egui::vec2(2.0, 2.0)), th.art_bg);
            let segs: usize = 12;
            let seg_h = vr.height() / segs as f32;
            let grid_col = Color32::from_rgba_unmultiplied(255, 255, 255, 12);
            for s in 1..segs {
                let y = vr.max.y - s as f32 * seg_h;
                painter.line_segment(
                    [egui::pos2(vr.min.x, y), egui::pos2(vr.max.x, y)],
                    egui::Stroke::new(1.0, grid_col),
                );
            }
            let bw = 11.0f32;
            let pitch = 15.0f32;
            let ox = vr.min.x + 2.0;
            for (i, &h) in self.viz_bars.iter().enumerate() {
                let x = ox + i as f32 * pitch;
                let lit = (segs as f32 * (h / vr.height()).clamp(0.0, 1.0)).round() as usize;
                for s in 0..segs {
                    let f = (s + 1) as f32 / segs as f32;
                    let r = egui::Rect::from_min_max(
                        egui::pos2(x, vr.max.y - f * vr.height()),
                        egui::pos2(x + bw, vr.max.y - f * vr.height() + seg_h),
                    );
                    let c = if s < lit {
                        if f > 0.78 { Color32::from_rgb(255, 70, 70) }
                        else if f > 0.58 { Color32::from_rgb(255, 170, 60) }
                        else if f > 0.38 { Color32::from_rgb(255, 220, 90) }
                        else { Color32::from_rgb(70, 230, 110) }
                    } else {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
                    };
                    painter.rect_filled(r, 1.0, c);
                }
                let ph = self.viz_peaks[i].min(vr.height()).max(0.0);
                let py = vr.max.y - ph;
                let pr = egui::Rect::from_min_size(egui::pos2(x - 1.0, py - 1.5), egui::vec2(bw + 2.0, 3.0));
                painter.rect_filled(pr, 1.5, Color32::from_rgb(240, 244, 255));
            }
            grill_dots(&painter, vr, Color32::from_rgba_unmultiplied(0, 0, 0, 40));
            ui.add_space(4.0);
            let playing_now = self.state.current_song.is_some() && !self.state.is_paused;
            let bass = self.viz_bars[0].max(self.viz_bars[1]);
            let pulse = if playing_now { ((bass - 8.0) / 68.0).clamp(0.0, 1.0).powf(1.2) } else { 0.0 };
            ui.horizontal(|ui| {
                speaker_woofer(ui, 76.0, th.btn_bg, th.accent, pulse);
                ui.add_space(10.0);
                speaker_woofer(ui, 76.0, th.btn_bg, th.accent, pulse);
            });
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                let v = self.state.volume as f32 / 100.0;
                let nv = rotary(ui, &format!("VOL {}%", self.state.volume), v, th.playing_fg, th.btn_bg);
                if (nv - v).abs() > 0.0001 {
                    self.set_volume((nv * 100.0).round() as u32);
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "-", th.btn_fg, th.btn_bg).clicked() { self.decrease_volume(); }
                        if retro_btn(ui, "+", th.btn_fg, th.btn_bg).clicked() { self.increase_volume(); }
                    });
                });
            });
            ui.add_space(4.0);
            ui.separator();
            let mut bands: Vec<String> = self.band_map.keys().cloned().collect();
            bands.sort_by_key(|b| b.to_lowercase());
            let mut sel = self.band_sel.clone();
            let mut all_clicked = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new("BANDS (A-Z)").color(darken(th.accent, 0.35)).monospace().size(8.0));
                if retro_btn(ui, "All", if sel.is_empty() { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() { sel = String::new(); all_clicked = true; }
                if retro_btn(ui, "Clear Bands", th.btn_fg, th.btn_bg).clicked() {
                    self.clear_bands();
                    sel = String::new();
                }
            });
            if bands.is_empty() {
                ui.label(RichText::new("Add music to build the band list").color(Color32::from_gray(90)).monospace().size(9.0));
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("band_scroll")
                    .auto_shrink([false, false])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut last_key = String::new();
                        for b in bands.iter() {
                            let ch = b.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('#');
                            let key = if ch.is_ascii_alphabetic() { ch.to_string() } else { "#".to_string() };
                            if key != last_key {
                                last_key = key.clone();
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new(&key).color(th.accent).monospace().strong().size(11.0));
                                });
                                ui.separator();
                            }
                            let n = self.band_map.get(b).map(|v| v.len()).unwrap_or(0);
                            let hit = sel == *b;
                            let text = format!("{}  ({})", b, n);
                            if ui.selectable_label(hit, RichText::new(text).color(if hit { th.playing_fg } else { th.fg }).monospace().size(10.0)).clicked() {
                                sel = b.clone();
                            }
                        }
                    });
                if all_clicked || self.band_sel != sel {
                    self.choose_band(if all_clicked { String::new() } else { sel });
                }
            }
            ui.add_space(4.0);
            self.draw_play_switch(ui, &th);
            ui.add_space(4.0);
            let pp = ui.painter().with_clip_rect(ui.clip_rect());
                let panel_right = ui.max_rect().right();
                let cy = ui.max_rect().center().y;
                let free = panel_right - ui.min_rect().right();
                if free > 40.0 {
                    print_sideways(
                        &pp,
                        panel_right - 1.0,
                        cy + 1.0,
                        "drDOOM69GAMING",
                        12.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 110),
                    );
                    print_sideways(
                        &pp,
                        panel_right - 7.0,
                        cy,
                        "drDOOM69GAMING",
                        12.0,
                        Color32::from_rgba_unmultiplied(255, 205, 120, 235),
                    );
                }
        });
    }

    fn draw_central(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::CentralPanel::default().show(ui, |ui| {
            let bg_rect = ui.available_rect_before_wrap();
            let bg = ui.interact(bg_rect, ui.id().with("bg_dbl_click"), egui::Sense::click());
            if bg.double_clicked() && !self.video_on {
                self.toggle_fullscreen();
            }
            let playing = self.state.current_song.is_some() && !self.state.is_paused;
            let (deck_plaque, deck_plaque_fg) = match self.deck_mode {
                DeckMode::Tape => ("  CASSETTE DECK  •  REV C", th.accent),
                DeckMode::Disc => ("  CD DECK  •  COMPACT DISC", th.playing_fg),
                DeckMode::Radio => ("  RADIO TUNER  •  FM / AM", Color32::from_rgb(255, 170, 80)),
            };
            plaque(ui, ui.available_width(), deck_plaque, deck_plaque_fg, th.bg);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("SW v{}", APP_VERSION)).color(darken(th.accent, 0.35)).monospace().size(8.0));
            });

            match self.deck_mode {
                DeckMode::Disc => {
                    ui.horizontal(|ui| {
                        let (tray, _) = ui.allocate_exact_size(egui::vec2(300.0, 108.0), egui::Sense::hover());
                        let p = ui.painter().with_clip_rect(tray);
                        p.rect_filled(tray, 6.0, darken(th.btn_bg, 0.4));
                        p.rect_stroke(tray, 6.0, egui::Stroke::new(1.5, darken(th.accent, 0.4)), egui::StrokeKind::Inside);
                        let slot = egui::Rect::from_min_max(
                            egui::pos2(tray.min.x + 14.0, tray.min.y + 20.0),
                            egui::pos2(tray.max.x - 14.0, tray.max.y - 16.0),
                        );
                        p.rect_filled(slot, 3.0, Color32::from_rgb(22, 24, 28));
                        p.rect_stroke(slot, 3.0, egui::Stroke::new(1.0, Color32::from_gray(46)), egui::StrokeKind::Inside);
                        if self.disc_in {
                            let t = ui.input(|i| i.time) as f32;
                            let c = slot.center();
                            let cx = 4.0;
                            let wave = (t * 5.0).sin() * cx;
                            let r = 30.0;
                            p.circle_filled(c + egui::vec2(wave * 0.15, 0.0), r + 1.0, Color32::from_gray(225));
                            p.circle_filled(c, r, Color32::from_gray(150));
                            p.circle_filled(c + egui::vec2(wave * 0.15, 0.0), r - 8.0, Color32::from_gray(64));
                            p.circle_filled(c, 3.5, Color32::from_gray(235));
                            let a = t * 3.0;
                            let shine = c + egui::vec2(a.cos(), a.sin()) * (r * 0.7);
                            p.circle_filled(shine, 4.5, Color32::from_rgba_unmultiplied(255, 255, 255, 80));
                            let _ = cx;
                            p.text(
                                egui::pos2(slot.center().x, slot.max.y - 8.0),
                                egui::Align2::CENTER_BOTTOM,
                                format!("{} TRACKS", self.state.playlist.len()),
                                egui::FontId::monospace(9.0),
                                Color32::from_gray(120),
                            );
                        } else {
                            p.text(
                                slot.center(),
                                egui::Align2::CENTER_CENTER,
                                "NO DISC",
                                egui::FontId::monospace(16.0),
                                Color32::from_gray(85),
                            );
                        }
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            if self.disc_in {
                                ui.label(RichText::new(&self.disc_label).color(th.playing_fg).monospace().strong().size(14.0));
                                ui.add_space(2.0);
                                ui.label(RichText::new(format!("{} TRACKS  •  CD", self.state.playlist.len())).color(Color32::from_gray(140)).monospace().size(10.0));
                                let now = self.state.current_song.clone().map(|s| stem(&s)).unwrap_or_else(|| "--".to_string());
                                ui.label(RichText::new(format!("♪ {}", now)).color(th.fg).monospace().size(11.0));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("POS").color(darken(th.accent, 0.35)).monospace().size(8.0));
                                    lcd(ui, &fmt_secs(self.pos), th.playing_fg, th.bg);
                                    ui.label(RichText::new("/").color(Color32::from_gray(110)).monospace().size(10.0));
                                    lcd(ui, &fmt_secs(self.len_secs), th.playing_fg, th.bg);
                                });
                            } else {
                                ui.label(RichText::new("NO DISC IN DECK").color(Color32::from_gray(120)).monospace().size(12.0));
                                ui.label(RichText::new("INSERT DISC  ->  load an album folder").color(Color32::from_gray(90)).monospace().size(10.0));
                            }
                            ui.add_space(7.0);
                            if retro_btn(ui, if self.disc_in { "⏏ EJECT DISC" } else { "▼ INSERT DISC" }, if self.disc_in { th.playing_fg } else { th.fg }, th.btn_bg).clicked() {
                                if self.disc_in { self.eject_disc(); } else { self.insert_disc(); }
                            }
                        });
                    });
                }
                DeckMode::Radio => {
                    ui.horizontal(|ui| {
                        let (lo, hi) = self.radio_band_range();
                        let span = hi - lo;
                        let dial_resp = ui.allocate_response(egui::vec2(ui.available_width().min(560.0), 132.0), egui::Sense::click_and_drag());
                        let rect = dial_resp.rect;
                        let p = ui.painter().with_clip_rect(rect.expand(2.0));
                        p.rect_filled(rect, 8.0, darken(th.btn_bg, 0.35));
                        p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, darken(th.btn_bg, 0.7)), egui::StrokeKind::Inside);
                        let screen = rect.shrink(8.0);
                        p.rect_filled(screen, 4.0, Color32::from_rgb(18, 20, 16));
                        p.rect_stroke(screen, 4.0, egui::Stroke::new(1.0, Color32::from_gray(40)), egui::StrokeKind::Inside);
                        let map = |f: f32| screen.min.x + screen.width() * ((f - lo) / span);
                        for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                            if s.freq >= lo && s.freq <= hi {
                                let x = map(s.freq);
                                p.line_segment(
                                    [egui::pos2(x, screen.max.y - 74.0), egui::pos2(x, screen.max.y - 62.0)],
                                    egui::Stroke::new(1.0, th.accent),
                                );
                            }
                        }
                        let n = 20;
                        for i in 0..=n {
                            let f = lo + span * (i as f32 / n as f32);
                            let x = map(f);
                            let major = i % 5 == 0;
                            let ty = if major { 30.0 } else { 14.0 };
                            p.line_segment(
                                [egui::pos2(x, screen.max.y - 56.0), egui::pos2(x, screen.max.y - 56.0 - ty)],
                                egui::Stroke::new(1.0, if major { Color32::from_gray(170) } else { Color32::from_gray(90) }),
                            );
                            if major {
                                p.text(
                                    egui::pos2(x, screen.max.y - 16.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    format!("{}", f.round()),
                                    egui::FontId::monospace(9.0),
                                    Color32::from_gray(120),
                                );
                            }
                        }
                        let nx = map(self.radio_freq).clamp(screen.min.x + 4.0, screen.max.x - 4.0);
                        let needle = if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) };
                        p.line_segment(
                            [egui::pos2(nx, screen.min.y + 4.0), egui::pos2(nx, screen.max.y - 4.0)],
                            egui::Stroke::new(2.0, needle),
                        );
                        let tri = [
                            egui::pos2(nx - 8.0, screen.max.y - 16.0),
                            egui::pos2(nx + 8.0, screen.max.y - 16.0),
                            egui::pos2(nx, screen.max.y - 4.0),
                        ];
                        p.add(egui::Shape::convex_polygon(tri.to_vec(), needle, egui::Stroke::NONE));
                        if dial_resp.dragged() {
                            let d = dial_resp.drag_delta().x / rect.width() * span;
                            self.radio_freq = (self.radio_freq + d).clamp(lo, hi);
                            self.radio_last_moved = ui.input(|i| i.time);
                        }
                        if dial_resp.clicked() {
                            if let Some(pos) = dial_resp.interact_pointer_pos() {
                                let f = lo + span * ((pos.x - screen.min.x) / screen.width());
                                self.radio_freq = f.clamp(lo, hi);
                                self.radio_last_moved = ui.input(|i| i.time);
                            }
                        }
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let fm_on = self.radio_band == "FM";
                                let am_on = self.radio_band == "AM";
                                if retro_btn(ui, "FM", if fm_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !fm_on {
                                    self.radio_band = "FM".to_string();
                                    self.radio_freq = 88.1;
                                    self.radio_last_moved = ui.input(|i| i.time);
                                }
                                if retro_btn(ui, "AM", if am_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !am_on {
                                    self.radio_band = "AM".to_string();
                                    self.radio_freq = 860.0;
                                    self.radio_last_moved = ui.input(|i| i.time);
                                }
                            });
                            ui.add_space(4.0);
                            lcd(ui, &fmt_freq(self.radio_freq), th.playing_fg, th.bg);
                            let stname = self.radio_tuned.and_then(|i| self.radio_presets.get(i)).map(|s| s.name.clone());
                            let playing = self.radio_on && !self.state.is_paused;
                            ui.label(RichText::new(stname.as_deref().unwrap_or("-- STATIC --")).color(if playing { th.playing_fg } else { Color32::from_gray(110) }).monospace().size(13.0));
                            if playing {
                                ui.label(RichText::new("● LIVE").color(Color32::from_rgb(0, 255, 90)).monospace().size(11.0));
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if retro_btn(ui, "TUNE-", th.btn_fg, th.btn_bg).clicked() {
                                    self.tune_radio_to(self.radio_freq - if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                                }
                                if retro_btn(ui, "SCAN", th.playing_fg, th.btn_bg).clicked() {
                                    self.scan_radio(ui.input(|i| i.time));
                                }
                                if retro_btn(ui, "TUNE+", th.btn_fg, th.btn_bg).clicked() {
                                    self.tune_radio_to(self.radio_freq + if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                                }
                                if retro_btn(ui, if playing { "PAUSE" } else { "PLAY" }, th.playing_fg, th.btn_bg).clicked() {
                                    if self.state.is_paused { self.unpause_music(); } else { self.pause_music(); }
                                }
                                if retro_btn(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                                    self.stop_radio();
                                }
                                if retro_btn(ui, "REC", if self.recording { th.playing_fg } else { Color32::from_rgb(255, 120, 120) }, mix(th.btn_bg, Color32::from_rgb(205, 45, 45), 0.35)).clicked() {
                                    self.record_radio();
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                let rows: Vec<(usize, f32, String)> = self
                                    .radio_presets
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band))
                                    .map(|(i, s)| (i, s.freq, s.name.clone()))
                                    .collect();
                                for (idx, freq, name) in rows {
                                    let current = self.radio_playing_idx == Some(idx);
                                    if ui.selectable_label(current, RichText::new(format!("{} {}", fmt_freq(freq), name)).color(if current { th.playing_fg } else { Color32::from_gray(150) }).monospace().size(9.0)).clicked() {
                                        self.tune_radio_to(freq, ui.input(|i| i.time));
                                        self.start_radio(idx);
                                    }
                                }
                            });
                        });
                    });
                }
                DeckMode::Tape => {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("COUNTER").color(darken(th.accent, 0.35)).monospace().size(8.0));
                            lcd(ui, &fmt_secs(self.pos), th.playing_fg, th.bg);
                            ui.add_space(2.0);
                            lcd(ui, &fmt_secs(self.len_secs), th.playing_fg, th.bg);
                        });
                        ui.add_space(10.0);
                        let wind = if self.winding.is_some() { 1.0 } else { 0.0 };
                        let wind_dir = self.winding.unwrap_or(0.0);
                        cassette(
                            ui,
                            (ui.available_width() - 12.0).max(150.0),
                            66.0,
                            self.tape_anim,
                            self.tape_side,
                            self.tape_songs,
                            self.tape_cap,
                            th.playing_fg,
                            th.btn_bg,
                            playing,
                            wind,
                            wind_dir,
                        );
                    });

                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("TUNE").color(th.accent).monospace().size(10.0));
                        let max = self.len_secs.max(0.1);
                        let mut p = self.pos;
                        let dial = ui.add(
                            egui::Slider::new(&mut p, 0.0..=max)
                                .show_value(false)
                                .trailing_fill(true),
                        );
                        if dial.dragged() {
                            self.pos = p;
                            self.dragging = true;
                        } else if self.dragging && dial.drag_stopped() {
                            self.set_status(format!("TUNE seek | pos={:.1} len={:.1} song={}", p, self.len_secs, self.state.current_song.as_deref().unwrap_or("NONE")));
                            self.dragging = false;
                            self.seek_to(p);
                            self.pos = p;
                        }
                        let dr = dial.rect;
                        let painter = ui.painter().with_clip_rect(dr.expand(2.0));
                        let n = 12;
                        for i in 0..=n {
                            let x = dr.min.x + dr.width() * (i as f32 / n as f32);
                            let len = if i % 3 == 0 { 6.0 } else { 2.5 };
                            painter.line_segment(
                                [egui::pos2(x, dr.center().y - len), egui::pos2(x, dr.center().y + len)],
                                egui::Stroke::new(1.0, darken(th.accent, 0.55)),
                            );
                        }
                        let frac = if max > 0.0 { (self.pos / max).clamp(0.0, 1.0) } else { 0.0 };
                        let nx = dr.min.x + dr.width() * frac;
                        painter.line_segment(
                            [egui::pos2(nx, dr.min.y + 1.0), egui::pos2(nx, dr.max.y - 1.0)],
                            egui::Stroke::new(1.0, th.playing_fg),
                        );
                        painter.text(
                            egui::pos2(dr.center().x, dr.max.y + 5.0),
                            egui::Align2::CENTER_TOP,
                            if self.len_secs > 0.0 { format!("{:>2}%", (self.pos / max * 100.0).round() as u32) } else { String::from("--%") },
                            egui::FontId::monospace(8.0),
                            darken(th.accent, 0.4),
                        );
                        ui.add_space(10.0);
                    });
                }
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new("DECK").color(darken(th.accent, 0.4)).monospace().size(9.0));
                let modes = [(DeckMode::Tape, "TAPE"), (DeckMode::Disc, "CD"), (DeckMode::Radio, "RADIO")];
                for (m, lbl) in modes {
                    let on = self.deck_mode == m;
                    if retro_btn(ui, lbl, if on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                        self.set_deck_mode(m);
                    }
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                let red = Color32::from_rgb(150, 26, 26);
                let rec_red = Color32::from_rgb(205, 45, 45);
                let wn = ui.input(|i| i.time);
                let rew = deck_key(ui, "REW", th.btn_fg, th.btn_bg);
                if rew.drag_started() {
                    self.wind_arm = Some((-1.0, wn));
                }
                if deck_key(ui, "PLAY", Color32::from_rgb(255, 235, 235), mix(th.btn_bg, red, 0.45)).clicked() {
                    if self.state.is_paused { self.unpause_music(); } else { self.pause_music(); }
                }
                let fwd = deck_key(ui, "FWD", th.btn_fg, th.btn_bg);
                if fwd.drag_started() {
                    self.wind_arm = Some((1.0, wn));
                }
                if let Some((d, t)) = self.wind_arm {
                    if self.winding.is_none() && wn - t >= 0.25 {
                        self.set_winding(Some(d));
                    }
                }
                if rew.clicked() && self.winding.is_none() {
                    self.seek_skip(-10.0);
                    self.set_status(format!("REW -10s | pos={:.1}", self.pos));
                }
                if fwd.clicked() && self.winding.is_none() {
                    self.seek_skip(10.0);
                    self.set_status(format!("FWD +10s | pos={:.1}", self.pos));
                }
                if rew.drag_stopped() {
                    self.wind_arm.take();
                    if self.winding == Some(-1.0) {
                        self.set_winding(None);
                    }
                }
                if fwd.drag_stopped() {
                    self.wind_arm.take();
                    if self.winding == Some(1.0) {
                        self.set_winding(None);
                    }
                }
                if deck_key(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                    self.stop_music();
                    if self.radio_on {
                        self.stop_radio();
                    }
                }
                if deck_key(ui, "PAUSE", th.btn_fg, th.btn_bg).clicked() {
                    self.pause_music();
                }
                let rec_lbl = if self.recording && (ui.input(|i| i.time) * 2.0).fract() < 0.5 {
                    "● REC"
                } else {
                    "REC"
                };
                if deck_key(ui, rec_lbl, Color32::from_rgb(255, 160, 160), mix(th.btn_bg, rec_red, 0.35)).clicked() {
                    self.req_record();
                }
                if deck_key(ui, "EJECT", th.btn_fg, th.btn_bg).clicked() {
                    if self.tape_out {
                        self.tape_out = false;
                        self.set_status("Tape in");
                    } else {
                        self.tape_out = true;
                        self.tape_reversing = false;
                        self.stop_music();
                        if self.radio_on {
                            self.stop_radio();
                        }
                        self.set_status("EJECT");
                    }
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "Skip", th.playing_fg, th.btn_bg).clicked() { self.skip_song(); }
                        if retro_btn(ui, "Prev", th.btn_fg, th.btn_bg).clicked() { self.prev_song(); }
                    });
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "Change Dir", th.playing_fg, th.btn_bg).clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.set_status("Scanning library...");
                                let _ = self.lib_tx.send(LibCmd::Scan(dir.to_string_lossy().to_string()));
                            }
                        }
                    });
                });
            });

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if retro_btn(ui, "Save", th.btn_fg, th.btn_bg).clicked() { self.save_song(); }
                if retro_btn(ui, "Clear", th.btn_fg, th.btn_bg).clicked() { self.clear_playlist(); }
                if retro_btn(ui, "Shuffle", th.playing_fg, th.btn_bg).clicked() { self.shuffle_playlist(); }
                if retro_btn(ui, "Smart", th.playing_fg, th.btn_bg).clicked() { self.smart_shuffle(); }
                if retro_btn(ui, "Add", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &AUDIO_FORMATS).pick_files() {
                        let entries: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
                        self.set_status("Adding files...");
                        let _ = self.lib_tx.send(LibCmd::AddMany(entries));
                    }
                }
            });

            ui.horizontal(|ui| {
                let accent = |on: bool| if on { th.playing_fg } else { th.btn_fg };
                if retro_btn(ui, &format!("Repeat: {}", if self.state.repeat_enabled { "On" } else { "Off" }), accent(self.state.repeat_enabled), th.btn_bg).clicked() { self.toggle_repeat(); }
                if retro_btn(ui, &format!("Playlist Only: {}", if self.state.playlist_only_mode { "On" } else { "Off" }), accent(self.state.playlist_only_mode), th.btn_bg).clicked() { self.toggle_playlist_only(); }
                if retro_btn(ui, &format!("Sequential: {}", if self.state.playlist_only_sequential { "On" } else { "Off" }), accent(self.state.playlist_only_sequential), th.btn_bg).clicked() { self.toggle_playlist_sequential(); }
                if retro_btn(ui, &format!("Dir Seq: {}", if self.state.dir_sequential { "On" } else { "Off" }), accent(self.state.dir_sequential), th.btn_bg).clicked() { self.toggle_dir_sequential(); }
            });

            ui.horizontal(|ui| {
                if retro_btn(ui, "Save Pl", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_file_name("playlist.json").add_filter("JSON", &["json"]).save_file() {
                        let pl = self.state.playlist.clone();
                        let _ = self.lib_tx.send(LibCmd::SavePlaylist(p.to_string_lossy().to_string(), pl));
                    }
                }
                if retro_btn(ui, "Load Pl", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                        let _ = self.lib_tx.send(LibCmd::LoadPlaylist(p.to_string_lossy().to_string()));
                    }
                }
                if retro_btn(ui, "Look Up", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(s) = self.state.current_song.clone() {
                        open_in_explorer(&s);
                    }
                }
                if retro_btn(ui, "Video", if self.video_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                    if self.video_on { self.close_video(); } else { self.play_video(); }
                }
                if retro_btn(ui, "Lyrics", th.btn_fg, th.btn_bg).clicked() { self.fetch_lyrics(); }
                if retro_btn(ui, "Help", th.btn_fg, th.btn_bg).clicked() { self.show_help = true; }
            });

            ui.horizontal_wrapped(|ui| {
                if retro_btn(ui, &format!("Theme: {}", self.state.theme), th.playing_fg, th.btn_bg).clicked() { self.cycle_theme(); }
                ui.label(RichText::new("EQ: DISABLED").color(darken(th.btn_fg, 0.35)).monospace().size(11.0));
                ui.separator();
                ui.label(RichText::new("Sleep:").color(th.fg).monospace().size(11.0));
                ui.add(egui::TextEdit::singleline(&mut self.sleep_input).desired_width(46.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                if retro_btn(ui, "Sleep", th.btn_fg, th.btn_bg).clicked() {
                    let minutes = self.sleep_input.trim().parse::<i64>().unwrap_or(0);
                    self.set_sleep_timer(minutes);
                }
                let sleep_label = if self.sleep_minutes > 0 { format!("Timer: {}m", self.sleep_minutes) } else { "Timer: Off".to_string() };
                lcd(ui, &sleep_label, if self.sleep_minutes > 0 { th.playing_fg } else { th.btn_fg }, th.bg);
            });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new("SEARCH:").color(th.fg).monospace().size(11.0));
                let resp = ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text("type to filter, click a row to play").desired_width(380.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                if resp.secondary_clicked() {
                    resp.request_focus();
                }
                resp.context_menu(|ui| {
                    if ui.button("Cut").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut); ui.close(); }
                    if ui.button("Copy").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy); ui.close(); }
                    if ui.button("Paste").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste); ui.close(); }
                });
                if self.search_focus {
                    resp.request_focus();
                    self.search_focus = false;
                }
                if retro_btn(ui, "J", Color32::from_rgb(255, 240, 160), th.btn_bg).clicked() {
                    self.search_focus = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("YT:").color(th.playing_fg).monospace().size(11.0));
                let resp = ui.add(egui::TextEdit::singleline(&mut self.yt_query).hint_text("paste a YouTube URL or search term").desired_width(340.0).font(egui::TextStyle::Monospace).text_color(th.accent));
                if resp.secondary_clicked() {
                    resp.request_focus();
                }
                resp.context_menu(|ui| {
                    if ui.button("Cut").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut); ui.close(); }
                    if ui.button("Copy").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy); ui.close(); }
                    if ui.button("Paste").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste); ui.close(); }
                });
                if self.yt_focus {
                    resp.request_focus();
                    self.yt_focus = false;
                }
                if retro_btn(ui, "Download", Color32::from_rgb(255, 240, 160), th.btn_bg).clicked() {
                    let q = self.yt_query.clone();
                    if q.trim().is_empty() {
                        self.set_error("Enter a YouTube URL or search term");
                    } else {
                        self.yt_pct = 0.0;
                        self.yt_speed.clear();
                        self.yt_eta.clear();
                        self.set_status(format!("YouTube: saving to {}", music_folder().display()));
                        let _ = self.net_tx.send(NetCmd::YtDownload(q));
                    }
                }
                ui.label(RichText::new("-> ").color(th.btn_fg).monospace().size(10.0));
                let folder = music_folder().to_string_lossy().to_string();
                ui.label(RichText::new(truncate_mid(&folder, 46)).color(Color32::from_gray(120)).monospace().size(10.0));
            });

            let query = self.search_query.to_lowercase();
            let cur_song = self.state.current_song.clone();
            let cur_idx = self.playing_pl_idx
                .filter(|i| *i < self.state.playlist.len())
                .or_else(|| cur_song.as_deref().and_then(|c| self.state.playlist.iter().position(|p| p.as_str() == c)));
            let rows: Vec<(usize, String)> = self
                .state
                .playlist
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    if query.is_empty() {
                        return true;
                    }
                    let d = self.display_cache[*i].to_lowercase();
                    let path = p.to_lowercase();
                    d.contains(&query) || path.contains(&query)
                })
                .map(|(i, _)| (i, self.display_cache[i].clone()))
                .collect();

            let mut play_choice: Option<usize> = None;
            let row_h = 20.0;
            let sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
            sa.show_rows(ui, row_h, rows.len(), |ui, range| {
                    for i in range {
                        let (idx, disp) = &rows[i];
                        let is_current = cur_idx == Some(*idx);
                        let color = if is_current { th.playing_fg } else { th.fg };
                        let mark = if is_current { "▶" } else { " " };
                        let rich = RichText::new(format!("{} {:>3}  {}", mark, idx + 1, disp))
                            .color(color)
                            .monospace();
                        let resp = ui.selectable_label(is_current, rich);
                        if resp.clicked() && self.drag_from.is_none() {
                            play_choice = Some(*idx);
                        }
                        if resp.drag_started() {
                            self.drag_from = Some(*idx);
                            self.drag_active = true;
                        }
                        if self.drag_active {
                            if resp.hovered() && self.drag_from != Some(*idx) {
                                self.drag_hover = Some(*idx);
                            }
                            if resp.drag_stopped() {
                                let from = self.drag_from;
                                let to = self.drag_hover;
                                self.drag_from = None;
                                self.drag_hover = None;
                                self.drag_active = false;
                                if let (Some(f), Some(t)) = (from, to) {
                                    if f != t {
                                        self.move_playlist(f, t);
                                    }
                                }
                            }
                            if self.drag_hover == Some(*idx) {
                                ui.painter().hline(
                                    resp.rect.min.x..=resp.rect.max.x,
                                    resp.rect.bottom(),
                                    egui::Stroke::new(2.0, th.accent),
                                );
                            }
                        }
                    }
                });
            if let Some(i) = play_choice {
                let p = self.state.playlist[i].clone();
                self.playing_pl_idx = Some(i);
                self.play_song(&p);
            }

            ui.add_space(4.0);
            let log_frame = egui::Frame::default()
                .fill(Color32::from_rgb(4, 4, 6))
                .stroke(egui::Stroke::new(1.0, th.btn_fg))
                .inner_margin(egui::Margin::symmetric(6, 3));
            log_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("[YT LOG]").color(th.accent).monospace().size(10.0));
                    if !self.yt_log.is_empty() && retro_btn(ui, "Clear", th.btn_fg, th.btn_bg).clicked() {
                        self.yt_log.clear();
                    }
                });
                if self.yt_pct > -1.0 {
                    let status = if self.yt_speed.is_empty() {
                        "starting...".to_string()
                    } else {
                        format!("{}  ETA {}", self.yt_speed, self.yt_eta)
                    };
                    ui.add(egui::ProgressBar::new(self.yt_pct / 100.0)
                        .desired_width(f32::INFINITY)
                        .text(RichText::new(status).color(Color32::from_gray(200)).monospace().size(10.0))
                        .fill(Color32::from_rgb(28, 224, 255)));
                }
                egui::ScrollArea::vertical().max_height(80.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if self.yt_log.is_empty() {
                        ui.label(RichText::new("idle - paste a YouTube URL or search term above").color(Color32::from_gray(90)).monospace().size(10.0));
                    } else {
                        for line in self.yt_log.iter().rev().take(24).rev() {
                            ui.label(RichText::new(truncate_mid(line, 130)).color(if line.starts_with("ERROR") || line.starts_with("Deleting") { Color32::from_rgb(255, 120, 120) } else { Color32::from_gray(170) }).monospace().size(10.0));
                        }
                    }
                });
            });
        });
    }

    fn show_radio_window(&mut self, ctx: &egui::Context) {
        let th = self.theme_colors();
        let mut open = self.show_radio;
        egui::Window::new("RADIO TUNER")
            .resizable(true)
            .collapsible(false)
            .default_width(430.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BAND:").color(th.fg).monospace().size(11.0));
                    let fm_on = self.radio_band == "FM";
                    let am_on = self.radio_band == "AM";
                    if retro_btn(ui, "FM", if fm_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !fm_on {
                        self.radio_band = "FM".to_string();
                        self.radio_freq = 88.1;
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                    if retro_btn(ui, "AM", if am_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !am_on {
                        self.radio_band = "AM".to_string();
                        self.radio_freq = 860.0;
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                });
                ui.add_space(4.0);
                let (lo, hi) = self.radio_band_range();
                let span = hi - lo;
                let dial_resp = ui.allocate_response(egui::vec2(ui.available_width().min(360.0), 150.0), egui::Sense::click_and_drag());
                let rect = dial_resp.rect;
                let p = ui.painter().with_clip_rect(rect.expand(2.0));
                p.rect_filled(rect, 8.0, darken(th.btn_bg, 0.35));
                p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, darken(th.btn_bg, 0.7)), egui::StrokeKind::Inside);
                let screen = rect.shrink(8.0);
                p.rect_filled(screen, 4.0, darken(Color32::from_rgb(18, 20, 16), 0.0));
                p.rect_stroke(screen, 4.0, egui::Stroke::new(1.0, Color32::from_gray(40)), egui::StrokeKind::Inside);
                let map = |f: f32| screen.min.x + screen.width() * ((f - lo) / span);
                for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                    if s.freq >= lo && s.freq <= hi {
                        let x = map(s.freq);
                        p.line_segment(
                            [egui::pos2(x, screen.min.y + 8.0), egui::pos2(x, screen.min.y + 20.0)],
                            egui::Stroke::new(1.0, th.accent),
                        );
                    }
                }
                let n = 20;
                for i in 0..=n {
                    let f = lo + span * (i as f32 / n as f32);
                    let x = map(f);
                    let major = i % 5 == 0;
                    let ty = if major { 30.0 } else { 16.0 };
                    p.line_segment(
                        [egui::pos2(x, screen.min.y + 26.0), egui::pos2(x, screen.min.y + 26.0 + ty)],
                        egui::Stroke::new(1.0, if major { Color32::from_gray(170) } else { Color32::from_gray(90) }),
                    );
                    if major {
                        let lab = if self.radio_band == "AM" { format!("{}", f.round()) } else { format!("{}", f.round()) };
                        p.text(
                            egui::pos2(x, screen.min.y + 60.0),
                            egui::Align2::CENTER_TOP,
                            lab,
                            egui::FontId::monospace(9.0),
                            Color32::from_gray(120),
                        );
                    }
                }
                for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                    if s.freq >= lo && s.freq <= hi {
                        let x = map(s.freq);
                        p.text(
                            egui::pos2(x, screen.min.y + 70.0),
                            egui::Align2::CENTER_TOP,
                            &s.name,
                            egui::FontId::monospace(7.5),
                            Color32::from_gray(150),
                        );
                    }
                }
                let nx = map(self.radio_freq).clamp(screen.min.x + 4.0, screen.max.x - 4.0);
                p.line_segment(
                    [egui::pos2(nx, screen.min.y + 4.0), egui::pos2(nx, screen.max.y - 4.0)],
                    egui::Stroke::new(1.6, if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) }),
                );
                let tri = [
                    egui::pos2(nx - 6.0, screen.max.y - 12.0),
                    egui::pos2(nx + 6.0, screen.max.y - 12.0),
                    egui::pos2(nx, screen.max.y - 3.0),
                ];
                p.add(egui::Shape::convex_polygon(tri.to_vec(), if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) }, egui::Stroke::NONE));
                ui.add_space(2.0);
                if dial_resp.dragged() {
                    let d = dial_resp.drag_delta().x / rect.width() * span;
                    self.radio_freq = (self.radio_freq + d).clamp(lo, hi);
                    self.radio_last_moved = ui.input(|i| i.time);
                }
                if dial_resp.clicked() {
                    if let Some(pos) = dial_resp.interact_pointer_pos() {
                        let f = lo + span * ((pos.x - screen.min.x) / screen.width());
                        self.radio_freq = f.clamp(lo, hi);
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                }
                let stname = self.radio_tuned.and_then(|i| self.radio_presets.get(i)).map(|s| s.name.clone());
                let playing = self.radio_on && !self.state.is_paused;
                ui.horizontal(|ui| {
                    lcd(ui, &fmt_freq(self.radio_freq), th.playing_fg, th.bg);
                    ui.label(RichText::new(stname.as_deref().unwrap_or("-- STATIC --")).color(if playing { th.playing_fg } else { Color32::from_gray(110) }).monospace().size(13.0));
                    if playing {
                        ui.label(RichText::new("● LIVE").color(Color32::from_rgb(0, 255, 90)).monospace().size(11.0));
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if retro_btn(ui, "TUNE-", th.btn_fg, th.btn_bg).clicked() {
                        self.tune_radio_to(self.radio_freq - if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                    }
                    if retro_btn(ui, "TUNE+", th.btn_fg, th.btn_bg).clicked() {
                        self.tune_radio_to(self.radio_freq + if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                    }
                    if retro_btn(ui, "SCAN", th.playing_fg, th.btn_bg).clicked() {
                        self.scan_radio(ui.input(|i| i.time));
                    }
                    if retro_btn(ui, if playing { "PAUSE" } else { "PLAY" }, th.playing_fg, th.btn_bg).clicked() {
                        if self.state.is_paused {
                            self.unpause_music();
                        } else {
                            self.pause_music();
                        }
                    }
                    if retro_btn(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                        self.stop_radio();
                    }
                    if retro_btn(ui, "REC", if self.recording { th.playing_fg } else { Color32::from_rgb(255, 120, 120) }, mix(th.btn_bg, Color32::from_rgb(205, 45, 45), 0.35)).clicked() {
                        self.record_radio();
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                let rows: Vec<(usize, f32, String)> = self
                    .radio_presets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band))
                    .map(|(i, s)| (i, s.freq, s.name.clone()))
                    .collect();
                egui::ScrollArea::vertical().max_height(140.0).show_rows(ui, 20.0, rows.len(), |ui, range| {
                    for i in range {
                        let (idx, freq, name) = &rows[i];
                        let current = self.radio_playing_idx == Some(*idx);
                        let lab = format!("{:>8}   {}", fmt_freq(*freq), name);
                        if ui.selectable_label(current, RichText::new(lab).color(if current { th.playing_fg } else { th.fg }).monospace()).clicked() {
                            self.tune_radio_to(*freq, ui.input(|i| i.time));
                            self.start_radio(*idx);
                        }
                    }
                });
            });
        self.show_radio = open;
    }

    fn show_help_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("SYSTEM MANUAL")
            .resizable(true)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                let th = self.theme_colors();
                let g = Color32::from_rgb(0, 255, 0);
                let dim = Color32::from_rgb(100, 200, 100);
                let acc = th.accent;

                ui.label(RichText::new("KEYBOARD SHORTCUTS").color(acc).monospace().strong());
                ui.label(RichText::new("SPACE .............. Play / Pause").color(g).monospace());
                ui.label(RichText::new("LEFT ................ Previous Track").color(g).monospace());
                ui.label(RichText::new("RIGHT ............... Skip / Next Track").color(g).monospace());
                ui.label(RichText::new("UP .................. Volume Up").color(g).monospace());
                ui.label(RichText::new("DOWN ................ Volume Down").color(g).monospace());
                ui.label(RichText::new("+  /  - ............. Volume Buttons").color(g).monospace());
                ui.label(RichText::new("S ................... Smart Shuffle (toggle)").color(g).monospace());
                ui.label(RichText::new("T ................... Cycle Theme").color(g).monospace());
                ui.label(RichText::new("F ................... Toggle Fullscreen").color(g).monospace());
                ui.label(RichText::new("M ................... Mute / Unmute").color(g).monospace());
                ui.label(RichText::new("L ................... Toggle Lyrics").color(g).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (TAPE MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("PLAY ................ Start / Pause playback").color(dim).monospace());
                ui.label(RichText::new("STOP ................ Stop playback").color(dim).monospace());
                ui.label(RichText::new("PAUSE ............... Pause playback").color(dim).monospace());
                ui.label(RichText::new("REW ................. Press & hold to rewind").color(dim).monospace());
                ui.label(RichText::new("FWD ................. Press & hold to fast-forward").color(dim).monospace());
                ui.label(RichText::new("SKIP ................ Next track").color(dim).monospace());
                ui.label(RichText::new("PREV ................ Previous track").color(dim).monospace());
                ui.label(RichText::new("REC ................. Record to file").color(dim).monospace());
                ui.label(RichText::new("EJECT ............... Insert / Eject tape").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (CD MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("INSERT DISC ......... Load music from folder").color(dim).monospace());
                ui.label(RichText::new("EJECT DISC .......... Remove disc").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (RADIO MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("FM / AM ............. Switch band").color(dim).monospace());
                ui.label(RichText::new("TUNE- / TUNE+ ........ Step frequency").color(dim).monospace());
                ui.label(RichText::new("SCAN ................ Auto-scan for stations").color(dim).monospace());
                ui.label(RichText::new("Click dial .......... Tune to frequency").color(dim).monospace());
                ui.label(RichText::new("Drag dial ........... Sweep frequencies").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("TUNE SLIDER").color(acc).monospace().strong());
                ui.label(RichText::new("Drag seek bar to scrub through the current song").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("PLAYLIST").color(acc).monospace().strong());
                ui.label(RichText::new("Click row ........... Play selected song").color(dim).monospace());
                ui.label(RichText::new("Drag & drop ......... Add files or folders").color(dim).monospace());
                ui.label(RichText::new("Band buttons ......... Filter by artist").color(dim).monospace());
                ui.label(RichText::new("ALL .................. Show all songs (A-Z)").color(dim).monospace());
                ui.label(RichText::new("Search bar ........... Filter playlist").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("EXTRAS").color(acc).monospace().strong());
                ui.label(RichText::new("Sleep Timer .......... Auto-close after time").color(dim).monospace());
                ui.label(RichText::new("Look Up ............. Open song folder in explorer").color(dim).monospace());
                ui.label(RichText::new("EQ .................. Equalizer presets").color(dim).monospace());
                ui.label(RichText::new("YouTube Download ..... Paste URL in status bar").color(dim).monospace());
                ui.label(RichText::new("Video Player ......... Double-click video in playlist").color(dim).monospace());
                ui.add_space(8.0);

                if retro_btn(ui, "CLOSE", Color32::from_rgb(0, 255, 0), th.btn_bg).clicked() {
                    self.show_help = false;
                }
            });
    }

    fn ui_lyrics(&mut self) {
        let ctx = self.ctx.clone();
        egui::Window::new(format!("Lyrics - {}", self.lyrics_title))
            .resizable(true)
            .collapsible(false)
            .default_size([520.0, 440.0])
            .show(&ctx, |ui| {
                if let Some(lyr) = self.lyrics.clone() {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.label(RichText::new(lyr).color(Color32::from_rgb(0x39, 0xFF, 0x14)).monospace());
                    });
                }
                let th = self.theme_colors();
                if retro_btn(ui, "CLOSE", Color32::from_rgb(0, 255, 0), th.btn_bg).clicked() {
                    self.lyrics = None;
                }
            });
    }

    fn fetch_lyrics(&mut self) {
        if let Some(s) = self.state.current_song.clone() {
            let (mut artist, mut title) = (self.meta.1.clone(), self.meta.0.clone());
            if artist.is_empty() || artist == "Unknown" {
                if let Some(idx) = title.find(" - ") {
                    let a = title[..idx].trim();
                    let t = title[idx + 3..].trim();
                    if !a.is_empty() && !t.is_empty() {
                        artist = a.to_string();
                        title = t.to_string();
                    }
                }
            }
            self.set_status("Fetching lyrics...");
            let _ = self.net_tx.send(NetCmd::Lyrics(artist, title));
            let _ = s;
        }
    }

    fn play_video(&mut self) {
        if let Some(song) = self.state.current_song.clone() {
            let song_path = PathBuf::from(&song);
            let dir = song_path.parent().map(|p| p.to_path_buf());
            let name = song_path.file_stem().map(|s| s.to_string_lossy().to_string());
            if let (Some(dir), Some(name)) = (dir, name) {
                for ext in ["mp4", "mkv", "avi", "webm"] {
                    let vp = dir.join(format!("{}.{}", name, ext));
                    if vp.is_file() {
                        self.enqueue_videos(vec![vp.to_string_lossy().to_string()]);
                        return;
                    }
                }
            }
        }
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Video", &["mp4", "mkv", "avi", "webm", "mov"])
            .pick_files()
        {
            let vids: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
            self.enqueue_videos(vids);
        }
    }

    fn attach_video_audio(&mut self, path: &str, seek: f32) {
        match spawn_video_audio(path, &ffmpeg_path(), seek) {
            Ok((mut child, pipe)) => {
                if let Ok(sink) = Sink::try_new(&self._stream_handle) {
                    sink.set_volume(self.video_volume as f32 / 100.0 * 0.9);
                    sink.append(pipe);
                    if !self.video_paused {
                        if self.video_tex.is_some() {
                            sink.play();
                        } else {
                            self.video_audio_pending = true;
                        }
                    }
                    self.video_audio_proc = Some(child);
                    self.video_sink = Some(sink);
                } else {
                    let _ = child.kill();
                }
            }
            Err(e) => {
                self.video_audio_pending = false;
                self.set_status(format!("MOVIE (video only): {}", e));
            }
        }
    }

    fn seek_video(&mut self, tsecs: f32) {
        let Some(path) = self.video_current.clone() else { return };
        let mut tsecs = tsecs.max(0.0);
        if self.video_dur_secs > 0.0 {
            tsecs = tsecs.min((self.video_dur_secs - 1.0).max(0.0));
        }
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
        self.video_gen += 1;
        let gen = self.video_gen;
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: tsecs });
        self.video_closing = false;
        self.video_paused = false;
        self.video_ended = false;
        self.video_cur_secs = tsecs;
        self.video_tex = None;
        self.attach_video_audio(&path, tsecs);
        let frac = if self.video_dur_secs > 0.0 { (tsecs / self.video_dur_secs).clamp(0.0, 1.0) } else { 0.0 };
        self.set_status(format!("MOVIE seek {:.0}%", frac * 100.0));
    }

    fn play_video_item(&mut self, path: String) {
        self.video_closing = false;
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
        if self.deck_mode == DeckMode::Tape {
            if let Some(cur) = self.state.current_song.clone() {
                if Path::new(&cur).is_file() {
                    self.deck_resume = Some((cur, self.pos.max(0.0)));
                }
            }
        }
        self.stop_radio();
        self.stop_music();
        self.video_gen += 1;
        let gen = self.video_gen;
        let saved_pos = self.video_positions.get(&path).copied().unwrap_or(0.0);
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: saved_pos });
        self.video_aspect = self.video_aspects.get(&path).copied().unwrap_or(self.video_aspect_default);
        self.video_on = true;
        self.video_paused = false;
        self.video_ended = false;
        self.video_pos = egui::pos2(60.0, 60.0);
        self.video_tex = None;
        self.video_bar_visible = false;
        self.video_dur_secs = 0.0;
        self.video_cur_secs = 0.0;
        self.video_seek_t = None;
        self.video_current = Some(path.clone());
        self.attach_video_audio(&path, saved_pos);
        self.video_cur_secs = saved_pos;
        self.set_status(format!("MOVIE {}/{}: {}", self.video_queue_idx + 1, self.video_queue.len(), stem(&path)));
        self.maybe_analyze_video(&path);
    }

    fn enqueue_videos(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        if self.video_on {
            let n = self.video_queue.len();
            self.video_queue.extend(paths);
            self.set_status(format!("Queued {} video(s) -> {}", self.video_queue.len() - n, self.video_queue.len()));
        } else {
            let s = self.video_queue.len();
            self.video_queue.extend(paths);
            let first = self.video_queue[s].clone();
            self.video_queue_idx = s;
            self.play_video_item(first);
        }
    }

    fn play_queue_idx(&mut self, idx: usize) {
        if idx < self.video_queue.len() {
            self.video_queue_idx = idx;
            self.play_video_item(self.video_queue[idx].clone());
        }
    }

    fn queue_next(&mut self) {
        if self.video_queue.is_empty() {
            return;
        }
        let ni = (self.video_queue_idx + 1) % self.video_queue.len();
        self.play_queue_idx(ni);
    }

    fn queue_prev(&mut self) {
        if self.video_queue.is_empty() {
            return;
        }
        let pi = (self.video_queue_idx + self.video_queue.len() - 1) % self.video_queue.len();
        self.play_queue_idx(pi);
    }

    fn remove_queue_item(&mut self, idx: usize) {
        if idx < self.video_queue.len() {
            self.video_queue.remove(idx);
            if self.video_queue.is_empty() {
                return;
            }
            if idx < self.video_queue_idx {
                self.video_queue_idx -= 1;
            }
            if self.video_queue_idx >= self.video_queue.len() {
                self.video_queue_idx = self.video_queue.len() - 1;
            }
        }
    }

    fn video_bound_secs(&self) -> (f32, f32) {
        let mut intro = if self.intro_skip_secs > 2.0 { self.intro_skip_secs } else { 90.0 };
        let mut credits = if self.video_dur_secs > 0.0 {
            (self.video_dur_secs - self.credits_skip_secs).max(0.0)
        } else {
            0.0
        };
        if let Some(p) = self.video_current.clone() {
            if let Some(&(ie, cs)) = self.video_bounds.get(&p) {
                if ie > 2.0 {
                    intro = ie;
                }
                if cs > 2.0 {
                    credits = cs;
                }
            } else if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                if let Some(&(ie, cs)) = self.show_bounds.get(&dir) {
                    if ie > 2.0 {
                        intro = ie;
                    }
                    if cs > 2.0 {
                        credits = cs;
                    }
                }
            }
        }
        if credits < 10.0 && self.video_dur_secs > 0.0 {
            credits = (self.video_dur_secs - self.credits_skip_secs).max(0.0);
        }
        (intro, credits)
    }

    fn maybe_analyze_video(&mut self, path: &str) {
        if !self.intro_skip_enabled {
            return;
        }
        let known = self
            .video_bounds
            .get(path)
            .map(|b| b.0 > 2.0 || b.1 > 2.0)
            .unwrap_or(false);
        if known || self.analyzing_video.contains(path) {
            return;
        }
        self.analyzing_video.insert(path.to_string());
        let gen = self.video_gen;
        let _ = self.lib_tx.send(LibCmd::VideoAnalyze { path: path.to_string(), gen });
        self.set_status(format!("ANALYZE intro/credits: {}", stem(path)));
    }

    fn start_video(&mut self, path: String) {
        if self.video_on {
            let n = self.video_queue.len();
            self.video_queue.push(path);
            self.set_status(format!("Queued -> {}", self.video_queue.len()));
            let _ = n;
        } else {
            self.video_queue.push(path);
            let last = self.video_queue.len() - 1;
            self.video_queue_idx = last;
            self.play_video_item(self.video_queue[last].clone());
        }
    }

    fn close_video(&mut self) {
        if let Some(path) = self.video_current.clone() {
            if self.video_cur_secs > 3.0 && self.video_dur_secs > 0.0 && self.video_cur_secs < self.video_dur_secs - 5.0 {
                self.video_positions.insert(path, self.video_cur_secs);
                self.save_settings();
            }
        }
        self.video_closing = true;
        let _ = self.lib_tx.send(LibCmd::VideoClose);
        self.video_on = false;
        self.video_ended = false;
        self.video_tex = None;
        self.set_video_fs(false);
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
    }

    fn toggle_video_play(&mut self) {
        if self.video_ended {
            self.video_ended = false;
            self.video_paused = false;
            if self.video_queue_idx < self.video_queue.len() {
                self.play_video_item(self.video_queue[self.video_queue_idx].clone());
                self.set_status("MOVIE: replay");
            }
            return;
        }
        self.video_paused = !self.video_paused;
        let _ = self.lib_tx.send(LibCmd::VideoPause(self.video_paused));
        if let Some(s) = &self.video_sink {
            if self.video_paused { s.pause(); } else { s.play(); }
        }
        self.set_status(if self.video_paused { "MOVIE: paused" } else { "MOVIE: playing" });
    }

    fn video_window(&mut self, ctx: &egui::Context) {
        let (w, h) = self.video_dims;
        if self.video_fs {
            let scr = ctx.viewport_rect();
            let sw = scr.width();
            let sh = scr.height();
            let (iw0, ih0) = if w > 0 && h > 0 { (w as f32, h as f32) } else { (16.0, 9.0) };
            let (iw, ih) = match self.video_aspect {
                VideoAspect::Original => (iw0, ih0),
                VideoAspect::R43 => (4.0, 3.0),
                VideoAspect::R169 => (16.0, 9.0),
            };
            let scale = (sw / iw).min(sh / ih);
            let vw = iw * scale;
            let vh = ih * scale;
            let img = egui::Rect::from_center_size(scr.center(), egui::vec2(vw, vh));
            let mouse = ctx.input(|i| i.pointer.latest_pos());
            let t_now = ctx.input(|i| i.time);
            let mouse_moved = ctx.input(|i| i.pointer.delta().length() > 0.0 || i.pointer.any_pressed());
            if self.video_last_mouse == 0.0 {
                self.video_last_mouse = t_now;
            }
            if mouse_moved {
                self.video_last_mouse = t_now;
            }
            let queue_region = egui::Rect::from_min_max(
                egui::pos2(scr.max.x - 300.0, scr.min.y + 32.0),
                egui::pos2(scr.max.x, scr.max.y),
            );
            let hovering = mouse.map_or(false, |m| {
                (m.y <= scr.min.y + 34.0 && m.x >= scr.min.x && m.x <= scr.max.x) || queue_region.contains(m)
            });
            self.video_bar_visible = hovering;
            let mut close = false;
            let mut togg = false;
            let mut next = false;
            let mut prev = false;
            let mut add = false;
            let mut aspect_cycle = false;
            let mut play_idx: Option<usize> = None;
            let mut del_idx: Option<usize> = None;
            let mut clearq = false;
            egui::Area::new(egui::Id::new("movie_win"))
                .fixed_pos(scr.min)
                .constrain(false)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let resp = ui.allocate_response(scr.size(), egui::Sense::click_and_drag());
                    let rect = resp.rect;
                    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                    if let Some(tex) = &self.video_tex {
                        ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "LOADING...", egui::FontId::monospace(14.0), Color32::from_gray(140));
                    }
                    if self.video_paused {
                        ui.painter().rect_stroke(img, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                    }
                    if self.video_bar_visible {
                        let bar = egui::Rect::from_min_size(scr.min, egui::vec2(sw, 30.0));
                        ui.painter().rect_filled(bar, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(8.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new("CLOSE")).clicked() {
                            close = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(104.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new(if self.video_paused { "PLAY" } else { "PAUSE" })).clicked() {
                            togg = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(200.0, 4.0), egui::vec2(56.0, 22.0)), egui::Button::new("<<")).clicked() {
                            prev = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(264.0, 4.0), egui::vec2(56.0, 22.0)), egui::Button::new(">>")).clicked() {
                            next = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(352.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new("+ ADD")).clicked() {
                            add = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(448.0, 4.0), egui::vec2(150.0, 22.0)), egui::Button::new(format!("ASPECT {}", self.video_aspect.label()))).clicked() {
                            aspect_cycle = true;
                        }
                        let skip_lbl = if self.intro_skip_enabled { "SKIP: ON" } else { "SKIP: OFF" };
                        let skip_color = if self.intro_skip_enabled { Color32::from_rgb(100, 255, 100) } else { Color32::from_gray(160) };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(606.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new(egui::RichText::new(skip_lbl).color(skip_color))).clicked() {
                            self.intro_skip_enabled = !self.intro_skip_enabled;
                            self.save_settings();
                            if self.intro_skip_enabled {
                                if let Some(p) = self.video_current.clone() {
                                    self.analyzing_video.remove(&p);
                                    self.maybe_analyze_video(&p);
                                }
                            }
                        }
                        let (bie, bcs) = self.video_bound_secs();
                        let bnd_lbl = if bie > 0.0 || bcs > 0.0 {
                            format!("INTRO {}", fmt_time(bie))
                        } else {
                            "INTRO --".to_string()
                        };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(700.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new(bnd_lbl)).clicked() {
                            if let Some(p) = self.video_current.clone() {
                                let (_, ccs) = self.video_bounds.get(&p).copied().unwrap_or((0.0, bcs));
                                self.video_bounds.insert(p.clone(), (self.video_cur_secs, ccs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    let (_, pc) = self.show_bounds.get(&dir).copied().unwrap_or((0.0, 0.0));
                                    self.show_bounds.insert(dir, (self.video_cur_secs, pc));
                                }
                                self.save_settings();
                                self.set_status(format!("Intro END set at {}", fmt_time(self.video_cur_secs)));
                            }
                        }
                        let crd_lbl = if bcs > 0.0 {
                            format!("CREDS {}", fmt_time(bcs))
                        } else {
                            "CREDS --".to_string()
                        };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(794.0, 4.0), egui::vec2(86.0, 22.0)), egui::Button::new(crd_lbl)).clicked() {
                            if let Some(p) = self.video_current.clone() {
                                let (iie, _) = self.video_bounds.get(&p).copied().unwrap_or((0.0, 0.0));
                                self.video_bounds.insert(p.clone(), (iie, self.video_cur_secs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    let (pi, _) = self.show_bounds.get(&dir).copied().unwrap_or((0.0, 0.0));
                                    self.show_bounds.insert(dir, (pi, self.video_cur_secs));
                                }
                                self.save_settings();
                                self.set_status(format!("Credits START set at {}", fmt_time(self.video_cur_secs)));
                            }
                        }
                        let mut vv = self.video_volume;
                        let vsl = ui.put(
                            egui::Rect::from_min_size(scr.min + egui::vec2(890.0, 8.0), egui::vec2(170.0, 14.0)),
                            egui::Slider::new(&mut vv, 0..=200).text("VOL"),
                        );
                        if vsl.changed() {
                            self.video_volume = vv;
                            if let Some(s) = &self.video_sink {
                                s.set_volume(vv as f32 / 100.0 * 0.9);
                            }
                            self.save_settings();
                        }
                    }
                    if self.video_bar_visible && !self.video_queue.is_empty() {
                        let qw = 300.0f32;
                        let qrect = egui::Rect::from_min_max(
                            egui::pos2(scr.max.x - qw, scr.min.y + 32.0),
                            egui::pos2(scr.max.x, scr.max.y),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(qrect), |ui| {
                            ui.painter().rect_filled(qrect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 190));
                            ui.painter().rect_stroke(qrect, 0.0, egui::Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Inside);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("QUEUE").color(Color32::from_rgb(120, 255, 120)).monospace().strong().size(12.0));
                                ui.label(RichText::new(format!(" {} ", self.video_queue.len())).color(Color32::from_gray(170)).monospace().size(11.0));
                                if ui.small_button("CLEAR").clicked() { clearq = true; }
                            });
                            ui.separator();
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                                let mut remove_here: Option<usize> = None;
                                let mut play_here: Option<usize> = None;
                                for (i, p) in self.video_queue.iter().enumerate() {
                                    let is_cur = i == self.video_queue_idx && self.video_on;
                                    ui.horizontal(|ui| {
                                        let col = if is_cur { Color32::from_rgb(255, 220, 80) } else { Color32::from_gray(200) };
                                        let mark = if is_cur { "▶" } else { " " };
                                        let txt = format!("{} {:02}  {}", mark, i + 1, stem(p));
                                        let label = RichText::new(txt).color(col).monospace().size(11.0);
                                        if is_cur {
                                            let row_rect = egui::Rect::from_min_size(
                                                ui.cursor().min,
                                                egui::vec2(ui.available_width(), 16.0),
                                            );
                                            ui.painter().rect_filled(
                                                egui::Rect::from_min_max(
                                                    egui::pos2(qrect.min.x, row_rect.min.y),
                                                    egui::pos2(qrect.max.x, row_rect.max.y),
                                                ),
                                                0.0,
                                                Color32::from_rgba_unmultiplied(255, 220, 80, 25),
                                            );
                                        }
                                        let resp = ui.add(egui::Label::new(label).truncate());
                                        if resp.clicked() {
                                            play_here = Some(i);
                                        }
                                        if ui.small_button("✕").clicked() {
                                            remove_here = Some(i);
                                        }
                                    });
                                }
                                if let Some(i) = play_here { play_idx = Some(i); }
                                if let Some(i) = remove_here { del_idx = Some(i); }
                            });
                        });
                    }
                    let bottom_hover = mouse.map_or(false, |m| m.y >= sh - 64.0 && m.y <= sh && m.x >= scr.min.x && m.x <= scr.max.x);
                    let show_seek = bottom_hover || self.video_seek_t.is_some();
                    if show_seek {
                        let by = sh - 66.0;
                        let lx = 40.0;
                        let rx = sw - 40.0;
                        let track = egui::Rect::from_min_max(egui::pos2(lx, by + 24.0), egui::pos2(rx, by + 30.0));
                        let yt = if let Some(t) = self.video_seek_t { t } else if self.video_dur_secs > 0.0 { (self.video_cur_secs / self.video_dur_secs).clamp(0.0, 1.0) } else { 0.0 };
                        let p = ui.painter();
                        p.rect_filled(egui::Rect::from_min_max(scr.min + egui::vec2(0.0, by), egui::pos2(scr.max.x, scr.max.y)), 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
                        p.rect_filled(track, 3.0, Color32::from_gray(60));
                        let fw = track.width() * yt;
                        if fw > 0.0 {
                            p.rect_filled(egui::Rect::from_min_size(track.min, egui::vec2(fw, track.height())), 3.0, Color32::from_rgb(255, 200, 60));
                        }
                        let tcol = Color32::from_rgb(225, 225, 225);
                        let shown_cur = if let Some(t) = self.video_seek_t { t * self.video_dur_secs } else { self.video_cur_secs };
                        p.text(egui::pos2(lx, by + 10.0), egui::Align2::LEFT_CENTER, fmt_time(shown_cur), egui::FontId::monospace(13.0), tcol);
                        p.text(egui::pos2(rx, by + 10.0), egui::Align2::RIGHT_CENTER, fmt_time(self.video_dur_secs), egui::FontId::monospace(13.0), tcol);
                        if self.video_dur_secs > 0.0 {
                            let mut t = yt;
                            let slip = ui.put(
                                egui::Rect::from_min_max(egui::pos2(lx, by), egui::pos2(rx, by + 34.0)),
                                egui::Slider::new(&mut t, 0.0..=1.0).show_value(false),
                            );
                            if slip.drag_started() || slip.dragged() {
                                self.video_seek_t = Some(t.clamp(0.0, 1.0));
                            }
                            if slip.drag_stopped() {
                                self.video_seek_t = None;
                                self.seek_video(t.clamp(0.0, 1.0) * self.video_dur_secs);
                            }
                        }
                    }
                    if resp.double_clicked() {
                        togg = true;
                    }
                });
            if let Some(i) = play_idx { self.play_queue_idx(i); }
            if let Some(i) = del_idx { self.remove_queue_item(i); }
            if clearq { self.video_queue.clear(); self.video_queue_idx = 0; }
            if add {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Video", &["mp4", "mkv", "avi", "webm", "mov"])
                    .pick_files()
                {
                    let vids: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
                    self.enqueue_videos(vids);
                }
            }
            if aspect_cycle {
                self.video_aspect = self.video_aspect.next();
                self.video_aspect_default = self.video_aspect;
                if let Some(p) = self.video_current.clone() {
                    self.video_aspects.insert(p, self.video_aspect);
                }
                self.save_settings();
                self.set_status(format!("MOVIE aspect {}: {}", self.video_aspect.label(), stem(self.video_current.as_deref().unwrap_or("-"))));
            }
            if prev { self.queue_prev(); }
            if next { self.queue_next(); }
            if togg {
                self.toggle_video_play();
            }
            if close {
                self.close_video();
            }
            self.video_hide_cursor(ctx, t_now);
            return;
        }
        let vw = 480.0f32;
        let vh = if h > 0 { vw * h as f32 / w as f32 } else { 270.0 };
        let mut close = false;
        let mut togg = false;
        egui::Area::new(egui::Id::new("movie_win"))
            .fixed_pos(self.video_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(Color32::from_rgb(4, 4, 6))
                    .stroke(egui::Stroke::new(2.0, Color32::from_rgb(96, 96, 108)))
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("MOVIE").color(self.theme_colors().accent).monospace().size(10.0));
                            ui.label(
                                RichText::new(if self.video_paused { "PAUSED - double click to resume" } else { "double click to pause" })
                                    .color(if self.video_paused { Color32::from_rgb(255, 220, 80) } else { Color32::from_rgb(120, 255, 120) })
                                    .monospace()
                                    .size(9.0),
                            );
                        });
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(vw, vh), egui::Sense::click_and_drag());
                        if let Some(tex) = &self.video_tex {
                            ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        } else {
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "LOADING...", egui::FontId::monospace(12.0), Color32::from_gray(120));
                        }
                        if self.video_paused {
                            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                        }
                        if resp.dragged() {
                            self.video_pos += resp.drag_delta();
                        }
                        if resp.double_clicked() {
                            togg = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("VOL").color(self.theme_colors().accent).monospace().size(10.0));
                            let mut vv = self.video_volume;
                            if ui.add(egui::Slider::new(&mut vv, 0..=200).show_value(false)).changed() {
                                self.video_volume = vv;
                                if let Some(s) = &self.video_sink {
                                    s.set_volume(vv as f32 / 100.0 * 0.9);
                                }
                                self.save_settings();
                            }
                            if ui.button("Close").clicked() { close = true; }
                            if ui.button(if self.video_paused { "Play" } else { "Pause" }).clicked() { togg = true; }
                            if ui.button("Fullscreen [Alt+Enter]").clicked() { self.set_video_fs(true); }
                        });
                    });
            });
        if togg {
            self.toggle_video_play();
        }
        if close {
            self.close_video();
        }
    }

    fn video_hide_cursor(&mut self, ctx: &egui::Context, t_now: f64) {
        if !self.video_fs || self.video_paused || self.video_ended || self.video_bar_visible {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
            return;
        }
        if t_now - self.video_last_mouse > 5.0 {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
    }
}

impl std::ops::Drop for PlayerApp {
    fn drop(&mut self) {
        self.close_video();
        self.stop_radio();
        self.save_settings();
        self.save_history();
        self.sink.stop();
    }
}

impl eframe::App for PlayerApp {
    fn on_exit(&mut self) {
        self.close_video();
        self.stop_radio();
        self.save_settings();
        self.save_history();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ctx = ctx.clone();
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        if !self.fs_asserted {
            self.fs_asserted = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        let now = ctx.input(|i| i.time);
        let dt = ((now - self.last_time) as f32).clamp(0.0, 0.1);
        self.last_time = now;
        self.step_tape(dt);
        self.step_wind(dt);
        self.apply_visuals(&self.ctx);
        ui.set_min_size(ui.available_size());
        let th = self.theme_colors();
        ui.painter().rect_filled(ui.max_rect(), 0.0, th.bg);

        self.drain_msg();
        self.update_playback_state();
        self.tick_radio(now);
        self.poll_ext();
        self.handle_keys();
        self.handle_drops();
        self.draw_sleep_timer(&self.ctx.clone());

        self.draw_title_bar(ui);
        self.draw_status_bar(ui);
        self.draw_side_panel(ui);
        self.draw_central(ui);

        if self.show_help {
            self.show_help_window(&ctx);
        }
        if self.show_radio {
            self.show_radio_window(&ctx);
        }
        if self.video_on {
            self.video_window(&ctx);
        }
        if self.lyrics.is_some() {
            self.ui_lyrics();
        }
    }
}

fn open_in_explorer(path: &str) {
    let dir = Path::new(path).parent().unwrap_or(Path::new("."));
    let _ = std::process::Command::new("explorer.exe").arg(dir.to_string_lossy().as_ref()).spawn();
}

fn make_tray() -> Result<TrayIcon, Box<dyn std::error::Error + Send + Sync>> {
    let menu = Menu::new();
    let show = MenuItem::with_id(MenuId::new("show"), "Show", true, None);
    let playpause = MenuItem::with_id(MenuId::new("play_pause"), "Play/Pause", true, None);
    let skip = MenuItem::with_id(MenuId::new("skip"), "Skip", true, None);
    let prev = MenuItem::with_id(MenuId::new("prev"), "Previous", true, None);
    let stop_item = MenuItem::with_id(MenuId::new("stop"), "Stop", true, None);
    let quit = MenuItem::with_id(MenuId::new("quit"), "Quit", true, None);
    let sep = PredefinedMenuItem::separator();
    menu.append_items(&[
        &show, &playpause, &skip, &prev, &stop_item, &sep, &quit,
    ])?;
    let icon = build_icon();
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(APP_NAME)
        .with_icon(icon)
        .build()?;
    Ok(tray)
}

fn build_icon() -> Icon {
    let mut rgba: Vec<u8> = vec![0; 32 * 32 * 4];
    let p1 = (9.0, 13.0);
    let p2 = (9.0, 25.0);
    let p3 = (24.0, 19.0);
    for y in 0..32 {
        for x in 0..32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - 16.0;
            let dy = py - 16.0;
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * 32 + x) * 4) as usize;
            let in_tri = point_in_tri(px, py, p1, p2, p3);
            if in_tri && d <= 14.5 {
                rgba[i] = 0; rgba[i + 1] = 255; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else if d <= 15.0 {
                rgba[i] = 42; rgba[i + 1] = 42; rgba[i + 2] = 42; rgba[i + 3] = 255;
            } else if d <= 16.5 {
                rgba[i] = 0; rgba[i + 1] = 170; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else {
                rgba[i] = 0; rgba[i + 1] = 0; rgba[i + 2] = 0; rgba[i + 3] = 0;
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).unwrap()
}

fn point_in_tri(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = cross(px, py, a, b);
    let d2 = cross(px, py, b, c);
    let d3 = cross(px, py, c, a);
    let neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(neg && pos)
}

fn cross(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
}

type HotkeyInfo = (GlobalHotKeyManager, u32, u32);

fn make_hotkeys() -> Option<HotkeyInfo> {
    let hkm = GlobalHotKeyManager::new().ok()?;
    let m_hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM);
    let q_hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyQ);
    if hkm.register(m_hk).is_err() {
        return None;
    }
    if hkm.register(q_hk).is_err() {
        let _ = hkm.unregister(m_hk);
        return None;
    }
    Some((hkm, m_hk.id(), q_hk.id()))
}

fn weighted_pick(pl: &[String], weights: &HashMap<String, f32>) -> String {
    let mut r = rand::random::<f64>();
    let total: f64 = pl.iter().map(|s| weights.get(s).copied().unwrap_or(1.0) as f64).sum();
    if total <= 0.0 { r = 0.0; } else { r *= total; }
    for s in pl {
        let w = weights.get(s).copied().unwrap_or(1.0) as f64;
        if r < w {
            return s.clone();
        }
        r -= w;
    }
    pl[0].clone()
}

fn random_next(current: Option<&str>, pl: &[String], weights: &HashMap<String, f32>) -> String {
    let s = weighted_pick(pl, weights);
    if pl.len() > 1 && current == Some(s.as_str()) {
        if let Some(i) = pl.iter().position(|p| *p == s) {
            return pl[(i + 1) % pl.len()].clone();
        }
    }
    s
}

fn open_decoder(path: &str) -> Option<Decoder<BufReader<File>>> {
    let f = File::open(path).ok()?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Decoder::new(BufReader::new(f)))) {
        Ok(Ok(d)) => Some(d),
        _ => None,
    }
}

fn make_display(path: &str) -> String {
    let folder = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{} - {}", folder, beautify_name(&stem(path)))
}

fn song_sort_key(path: &str) -> (String, String) {
    let s = stem(path);
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        let (title, artist) = if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                (&after[..open], after[open + 1..after.len() - 1].trim())
            } else {
                (after, "")
            }
        } else {
            (after, "")
        };
        let artist = if !artist.is_empty() {
            artist.to_string()
        } else {
            Path::new(path)
                .parent()
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        (
            artist.to_lowercase(),
            title.trim().to_lowercase(),
        )
    } else {
        let folder = Path::new(path)
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        (folder.to_lowercase(), s.to_lowercase())
    }
}

fn band_of(path: &str) -> String {
    let parent = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if parent.is_empty() {
        return parent;
    }
    if let Some(open) = parent.rfind('(') {
        if parent.ends_with(')') {
            let inner = parent[open + 1..parent.len() - 1].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    if let Some(pos) = parent.find(" - ") {
        return parent[..pos].trim().to_string();
    }
    parent
}

fn band_of_file(path: &str) -> String {
    let s = stem(path);
    if s.is_empty() {
        return band_of(path);
    }
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                let artist = after[open + 1..after.len() - 1].trim();
                if !artist.is_empty() {
                    return artist.to_string();
                }
            }
        }
        return s[..pos].trim().to_string();
    }
    band_of(path)
}

fn load_bands(path: &std::path::Path) -> HashMap<String, Vec<String>> {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, Vec<String>>>(&text) {
            return map;
        }
    }
    HashMap::new()
}

fn main() -> eframe::Result {
std::panic::set_hook(Box::new(|info| {
        use std::io::Write as _;
        let bt = std::backtrace::Backtrace::force_capture();
        let log = data_dir().join("crash.log");
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(f, "\n===== panic at {} =====", ts);
            let _ = writeln!(f, "{}", info);
            let _ = writeln!(f, "{}", bt);
        }
        eprintln!("{}", info);
    }));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(true)
            .with_title(format!("{} v{}", APP_NAME, APP_VERSION)),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            let app = PlayerApp::new(cc)?;
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

#[cfg(test)]
mod eq_dsp_tests {
    use super::*;
    use rodio::Source;

    fn rms(path: &Path) -> f64 {
        let mut r = hound::WavReader::open(path).expect("open wav");
        let samples: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|&x| (x as f64 / 32767.0).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    }

    #[test]
    fn encode_eq_changes_signal() {
        let dir = std::env::temp_dir().join("mediaplayerofdoom_eqtest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.wav");
        let rate = 44100u32;
        let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        {
            let mut w = hound::WavWriter::create(&src, spec).unwrap();
            let n = rate as usize * 3;
            for i in 0..n {
                let t = i as f64 / rate as f64;
                let v = (2.0 * std::f64::consts::PI * 80.0 * t).sin() * 0.3
                    + (2.0 * std::f64::consts::PI * 1500.0 * t).sin() * 0.3;
                w.write_sample((v * 32767.0) as i16).unwrap();
            }
        }
        let out = std::env::temp_dir().join("mediaplayerofdoom_eqtest.wav");
        let _ = std::fs::remove_file(&out);
        encode_eq(src.to_string_lossy().as_ref(), &[8.0, 6.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &out).expect("encode ok");
        assert!(out.is_file(), "eq output file exists");
        let a = rms(&src);
        let b = rms(&out);
        assert!(b > a * 1.5, "EQ should boost RMS: in={:.4} out={:.4}", a, b);

        let _ = std::fs::remove_file(&out);
        encode_eq(src.to_string_lossy().as_ref(), &[-8.0, -6.0, -4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &out).expect("cut ok");
        let c = rms(&out);
        assert!(c < a * 0.9, "EQ cut should reduce RMS: in={:.4} out={:.4}", a, c);

        let f = File::open(&out).expect("eq wav open");
        let dec = rodio::Decoder::new(BufReader::new(f)).expect("rodio decodes eq wav");
        let (sr, ch) = (dec.sample_rate(), dec.channels());
        assert!(sr > 0 && ch >= 1, "eq wav valid params");
        drop(dec);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wav_bool(bytes: &[u8]) -> bool {
        bytes.get(8..12) == Some("WAVE".as_bytes())
    }

    #[test]
    fn encode_eq_alters_real_song() {
        let real = match std::env::var("MPD_TEST_MP3") {
            Ok(p) if Path::new(&p).is_file() => p,
            _ => return,
        };
        let a1 = std::fs::read(&real).expect("read real song");
        let out = std::env::temp_dir().join("mediaplayerofdoom_eqtest_real.wav");
        let _ = std::fs::remove_file(&out);
        encode_eq(&real, &[6.0, 5.0, 3.0, -2.0, 2.0, 6.0, 7.0, 6.0, 5.0, 4.0], &out).expect("real song encodes");
        let a2 = std::fs::read(&out).expect("read eq output");
        assert!(!wav_bool(&a1), "source of a compressed song should not be a wav");
        assert!(wav_bool(&a2), "eq output is a wav file");
        let diff = rms(&out);
        assert!(diff > 0.01, "eq output has real audio content: rms={}", diff);
        let _ = std::fs::remove_file(&out);
    }
}