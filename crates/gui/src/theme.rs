//! rimdeck's look: a salvaged ship's manifest. Steel panels told apart by tone rather than
//! boxes, warm parchment text, verdigris for "this is selected / do this", and ochre/brick
//! only where something needs attention. Lexend is set for reading long lists comfortably.

use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Font, Theme};

pub const HULL: Color = Color::from_rgb8(0x26, 0x2B, 0x2F);
pub const PLATE: Color = Color::from_rgb8(0x30, 0x36, 0x3B);
pub const RIVET: Color = Color::from_rgb8(0x4A, 0x53, 0x5A);
pub const PARCHMENT: Color = Color::from_rgb8(0xE8, 0xE1, 0xCE);
pub const DUST: Color = Color::from_rgb8(0x9A, 0xA0, 0xA3);
pub const VERDIGRIS: Color = Color::from_rgb8(0x6F, 0xA8, 0xA0);
pub const OCHRE: Color = Color::from_rgb8(0xD8, 0xA2, 0x3E);
pub const BRICK: Color = Color::from_rgb8(0xCF, 0x5A, 0x4A);

pub const FONT: Font = Font::with_name("Lexend");
pub const MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..FONT
};
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..FONT
};

pub const FONT_FILES: [&[u8]; 3] = [
    include_bytes!("../fonts/Lexend-Regular.ttf"),
    include_bytes!("../fonts/Lexend-Medium.ttf"),
    include_bytes!("../fonts/Lexend-SemiBold.ttf"),
];

pub fn theme() -> Theme {
    Theme::custom(
        "rimdeck",
        Palette {
            background: HULL,
            text: PARCHMENT,
            primary: VERDIGRIS,
            success: VERDIGRIS,
            warning: OCHRE,
            danger: BRICK,
        },
    )
}

/// A list, detail, or issue surface: one step lighter than the window, no outline.
pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PLATE.into()),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn status_bar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PLATE.into()),
        text_color: Some(DUST),
        ..container::Style::default()
    }
}

pub fn selected_row(_: &Theme) -> container::Style {
    container::Style {
        background: Some(VERDIGRIS.scale_alpha(0.32).into()),
        border: Border {
            radius: 2.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

fn filled(fill: Color, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => fill.scale_alpha(0.85),
        button::Status::Pressed => fill.scale_alpha(0.7),
        button::Status::Disabled => fill.scale_alpha(0.35),
        button::Status::Active => fill,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: HULL,
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// The one or two actions that move the job forward (Sort, Save changes, Run game).
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    filled(VERDIGRIS, status)
}

/// Save while there are unsaved changes.
pub fn attention(_: &Theme, status: button::Status) -> button::Style {
    filled(OCHRE, status)
}

fn outlined(text_color: Color, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(PLATE.into()),
        _ => None,
    };
    button::Style {
        background,
        text_color: if status == button::Status::Disabled {
            DUST.scale_alpha(0.5)
        } else {
            text_color
        },
        border: Border {
            color: RIVET,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..button::Style::default()
    }
}

/// Everything else: tools, list actions, secondary choices.
pub fn quiet(_: &Theme, status: button::Status) -> button::Style {
    outlined(PARCHMENT, status)
}

/// Tools that are used now and then: text until hovered.
pub fn subtle(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..outlined(PARCHMENT, status)
    }
}

/// Destructive actions (delete, discard).
pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    outlined(BRICK, status)
}

/// Bare text buttons: tabs and inline ✕.
pub fn bare(text_color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: None,
        text_color: if status == button::Status::Hovered {
            PARCHMENT
        } else {
            text_color
        },
        ..button::Style::default()
    }
}

/// Tabs: the open one is a raised plate, the rest are quiet text.
pub fn tab(_: &Theme, status: button::Status, open: bool) -> button::Style {
    button::Style {
        background: open.then(|| PLATE.into()),
        text_color: if open || status == button::Status::Hovered {
            PARCHMENT
        } else {
            DUST
        },
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}
