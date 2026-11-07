//! Small, consistent outline glyphs rendered by Rune's native icon atlas.
use rune::Icon;

pub static AUTO: Icon = Icon {
    name: "auto-contrast",
    view: 24.0,
    body: r#"<path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4M8 15v-4M12 15V8M16 15v-6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
pub static PRESETS: Icon = Icon {
    name: "ct-presets",
    view: 24.0,
    body: r#"<path d="M4 6h16M4 12h16M4 18h16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/><path d="M8 4v4M16 10v4M10 16v4" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round"/>"#,
};

pub static SPLIT: Icon = Icon {
    name: "split-panes",
    view: 24.0,
    body: r#"<rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M12 4v16" fill="none" stroke="currentColor" stroke-width="1.7"/>"#,
};

pub static SIDEBAR: Icon = Icon {
    name: "sidebar",
    view: 24.0,
    body: r#"<rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M9 4v16" fill="none" stroke="currentColor" stroke-width="1.7"/>"#,
};
pub static EYE: Icon = Icon {
    name: "eye",
    view: 24.0,
    body: r#"<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="1.7"/>"#,
};
pub static EYE_OFF: Icon = Icon {
    name: "eye-off",
    view: 24.0,
    body: r#"<path d="m3 3 18 18M9 5.4A12 12 0 0 1 12 5c6.5 0 10 7 10 7a19 19 0 0 1-3.1 4M15 18.6A12 12 0 0 1 12 19C5.5 19 2 12 2 12a19 19 0 0 1 3.1-4M10 10a2.8 2.8 0 0 0 4 4" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

pub static FOLDER: Icon = Icon {
    name: "folder",
    view: 24.0,
    body: r#"<path d="M3 7V5h6l2 2h10v12H3Z M3 10h18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
pub static PAN: Icon = Icon {
    name: "pan",
    view: 24.0,
    body: r#"<path d="M12 3v18M3 12h18M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
pub static WINDOW: Icon = Icon {
    name: "window-level",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M12 4v16M12 4a8 8 0 0 1 0 16Z" fill="currentColor"/>"#,
};
pub static RESET: Icon = Icon {
    name: "reset",
    view: 24.0,
    body: r#"<path d="M4 4v6h6M4 10a8 8 0 1 1 1 8M12 8v4l3 2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
pub static INVERT: Icon = Icon {
    name: "invert",
    view: 24.0,
    body: r#"<rect x="4" y="4" width="16" height="16" rx="1" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M4 20 20 4M4 4h16L4 20Z" fill="currentColor"/>"#,
};
pub static ZOOM: Icon = Icon {
    name: "zoom",
    view: 24.0,
    body: r#"<circle cx="10" cy="10" r="6.5" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="m15 15 6 6M7 10h6M10 7v6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>"#,
};
pub static FIT: Icon = Icon {
    name: "fit",
    view: 24.0,
    body: r#"<path d="M8 3H3v5M16 3h5v5M21 16v5h-5M3 16v5h5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/><rect x="7" y="7" width="10" height="10" rx="1" fill="none" stroke="currentColor" stroke-width="1.7"/>"#,
};
pub static ROTATE: Icon = Icon {
    name: "rotate-clockwise",
    view: 24.0,
    body: r#"<path d="M20 5v5h-5M20 10a8 8 0 1 0-1 7" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
