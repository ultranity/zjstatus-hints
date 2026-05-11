use ansi_term::{
    ANSIString, ANSIStrings, Colour,
    Colour::{Fixed, RGB},
    Style,
};
use std::collections::{BTreeMap, HashMap};
use zellij_tile::prelude::actions::Action;
use zellij_tile::prelude::actions::SearchDirection;
use zellij_tile::prelude::*;

// ---------------------------------------------------------------------------
// Modifier style
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy, PartialEq)]
enum ModifierStyle {
    #[default]
    Long,
    Short,
    Symbol,
}

impl ModifierStyle {
    fn from_str(s: &str) -> Self {
        match s {
            "short" => Self::Short,
            "symbol" => Self::Symbol,
            _ => Self::Long,
        }
    }
}

const DEFAULT_MODIFIER_STYLE: ModifierStyle = ModifierStyle::Long;
/// Default `hint_format`. Bold key, plain action. Users override entirely with
/// the zjstatus inline style syntax: `"#[fg=#X,bold]{key}#[fg=#Y] {action}"`.
const DEFAULT_HINT_FORMAT: &str = "#[bold]{key}#[] {action}";
const DEFAULT_SEPARATOR: &str = "  ";

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    initialized: bool,
    pipe_name: String,
    mode_info: ModeInfo,
    base_mode_is_locked: bool,
    max_length: usize,
    overflow_str: String,
    hide_in_base_mode: bool,
    modifier_style: ModifierStyle,
    hint_format: String,
    separator: String,
    /// Maximum number of alternative keybindings shown per action (0 = unlimited).
    max_keys: usize,
    /// User-defined display aliases for action labels. Key = original label
    /// (e.g. `"split right"`), value = replacement string shown in the bar.
    action_aliases: HashMap<String, String>,
    /// User-defined display aliases for special bare keys.
    /// Key = lowercase key name (e.g. `"enter"`, `"space"`), value = replacement.
    key_aliases: HashMap<String, String>,
}

register_plugin!(State);

const TO_NORMAL: Action = Action::SwitchToMode { input_mode: InputMode::Normal };

const PLUGIN_SESSION_MANAGER: &str = "session-manager";
const PLUGIN_CONFIGURATION: &str = "configuration";
const PLUGIN_MANAGER: &str = "plugin-manager";
const PLUGIN_ABOUT: &str = "zellij:about";

const KEY_PATTERNS_NO_SEPARATOR: &[&str] = &["HJKL", "hjkl", "←↓↑→", "←→", "↓↑", "[]"];

const DEFAULT_MAX_LENGTH: usize = 0;
const DEFAULT_MAX_KEYS: usize = 0;
const DEFAULT_OVERFLOW_STR: &str = "...";
const DEFAULT_PIPE_NAME: &str = "zjstatus_hints";

type ActionLabel = (Action, &'static str);
type ActionSequenceLabel = (&'static [Action], &'static str);

const NORMAL_MODE_ACTIONS: &[ActionLabel] = &[
    (Action::SwitchToMode { input_mode: InputMode::Pane }, "pane"),
    (Action::SwitchToMode { input_mode: InputMode::Tab }, "tab"),
    (Action::SwitchToMode { input_mode: InputMode::Resize }, "resize"),
    (Action::SwitchToMode { input_mode: InputMode::Move }, "move"),
    (Action::SwitchToMode { input_mode: InputMode::Scroll }, "scroll"),
    (Action::SwitchToMode { input_mode: InputMode::Search }, "search"),
    (Action::SwitchToMode { input_mode: InputMode::Session }, "session"),
    (Action::Quit, "quit"),
];

const PANE_MODE_ACTION_SEQUENCES: &[ActionSequenceLabel] = &[
    (
        &[
            Action::NewPane { direction: None, pane_name: None, start_suppressed: false },
            TO_NORMAL,
        ],
        "new",
    ),
    (&[Action::CloseFocus, TO_NORMAL], "close"),
    (&[Action::ToggleFocusFullscreen, TO_NORMAL], "fullscreen"),
    (&[Action::ToggleFloatingPanes, TO_NORMAL], "float"),
    (&[Action::TogglePaneEmbedOrFloating, TO_NORMAL], "embed"),
    (
        &[
            Action::NewPane { direction: Some(Direction::Right), pane_name: None, start_suppressed: false },
            TO_NORMAL,
        ],
        "split right",
    ),
    (
        &[
            Action::NewPane { direction: Some(Direction::Down), pane_name: None, start_suppressed: false },
            TO_NORMAL,
        ],
        "split down",
    ),
];

const TAB_MODE_ACTION_SEQUENCES: &[ActionSequenceLabel] = &[
    (
        &[
            Action::NewTab {
                tiled_layout: None,
                floating_layouts: vec![],
                swap_tiled_layouts: None,
                swap_floating_layouts: None,
                tab_name: None,
                should_change_focus_to_new_tab: true,
                cwd: None,
                initial_panes: None,
                first_pane_unblock_condition: None,
            },
            TO_NORMAL,
        ],
        "new",
    ),
    (&[Action::CloseTab, TO_NORMAL], "close"),
    (&[Action::BreakPane, TO_NORMAL], "break pane"),
    (&[Action::ToggleActiveSyncTab, TO_NORMAL], "sync"),
];

// ---------------------------------------------------------------------------
// ZellijPlugin impl
// ---------------------------------------------------------------------------

fn get_common_modifiers(mut key_bindings: Vec<&KeyWithModifier>) -> Vec<KeyModifier> {
    if key_bindings.is_empty() {
        return vec![];
    }
    let mut common_modifiers = key_bindings.pop().unwrap().key_modifiers.clone();
    for key in key_bindings {
        common_modifiers = common_modifiers
            .intersection(&key.key_modifiers)
            .cloned()
            .collect();
    }
    common_modifiers.into_iter().collect()
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.initialized = false;

        // TODO: configuration validation
        self.max_length = configuration
            .get("max_length")
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_LENGTH);
        self.overflow_str = configuration
            .get("overflow_str")
            .cloned()
            .unwrap_or_else(|| DEFAULT_OVERFLOW_STR.to_string());
        self.pipe_name = configuration
            .get("pipe_name")
            .cloned()
            .unwrap_or_else(|| DEFAULT_PIPE_NAME.to_string());
        self.hide_in_base_mode = configuration
            .get("hide_in_base_mode")
            .map(|s| s.to_lowercase().parse::<bool>().unwrap_or(false))
            .unwrap_or(false);
        self.modifier_style = configuration
            .get("modifier_style")
            .map(|s| ModifierStyle::from_str(s.as_str()))
            .unwrap_or(DEFAULT_MODIFIER_STYLE);
        self.hint_format = configuration
            .get("hint_format")
            .cloned()
            .unwrap_or_else(|| DEFAULT_HINT_FORMAT.to_string());
        self.separator = configuration
            .get("separator")
            .cloned()
            .unwrap_or_else(|| DEFAULT_SEPARATOR.to_string());
        self.max_keys = configuration
            .get("max_keys")
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_KEYS);
        self.action_aliases = configuration
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("alias_")
                    .map(|label_raw| (label_raw.replace('_', " "), v.clone()))
            })
            .collect();
        self.key_aliases = configuration
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("key_alias_")
                    .map(|key_raw| (key_raw.to_lowercase(), v.clone()))
            })
            .collect();

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::MessageAndLaunchOtherPlugins,
        ]);

        // NOTE: do NOT call set_selectable(false) here. zellij shows the
        // permission grant dialog inside this plugin's pane; if the pane is
        // unfocusable the user cannot interact with the dialog. Defer the
        // call until permissions are confirmed (see `update`).
        subscribe(&[
            EventType::ModeUpdate,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = !self.initialized;
        match event {
            Event::ModeUpdate(mode_info) => {
                if self.mode_info != mode_info {
                    should_render = true;
                }
                self.mode_info = mode_info;
                self.base_mode_is_locked = self.mode_info.base_mode == Some(InputMode::Locked);
            }
            Event::PermissionRequestResult(_result) => {
                // Permissions resolved (granted or denied). Safe to hide the
                // plugin pane from selection now.
                set_selectable(false);
            }
            _ => {}
        };
        should_render
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        let mode_info = &self.mode_info;
        let output = if !(self.hide_in_base_mode && Some(mode_info.mode) == mode_info.base_mode) {
            let keymap = get_keymap_for_mode(mode_info);
            let parts = render_hints_for_mode(
                mode_info.mode,
                &keymap,
                self.modifier_style,
                &self.hint_format,
                &self.separator,
                self.max_keys,
                &self.action_aliases,
                &self.key_aliases,
            );

            let ansi_strings = ANSIStrings(&parts);
            let formatted = format!(" {}", ansi_strings);

            let visible_len = calculate_visible_length(&formatted);
            if self.max_length > 0 && visible_len > self.max_length {
                truncate_ansi_string(&formatted, &self.overflow_str, self.max_length)
            } else {
                formatted.to_string()
            }
        } else {
            String::new()
        };

        if !output.is_empty() {
            self.initialized = true;
        }

        pipe_message_to_plugin(MessageToPlugin::new("pipe").with_payload(format!(
            "zjstatus::pipe::pipe_{}::{}",
            self.pipe_name, output
        )));
        print!("{}", output);
    }
}

// ---------------------------------------------------------------------------
// ANSI helpers
// ---------------------------------------------------------------------------

struct AnsiParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> AnsiParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            chars: text.chars().peekable(),
        }
    }

    fn next_segment(&mut self) -> Option<AnsiSegment> {
        let ch = self.chars.next()?;

        if ch == '\x1b' {
            let mut escape_seq = String::from(ch);
            for escape_ch in self.chars.by_ref() {
                escape_seq.push(escape_ch);
                if escape_ch == 'm' {
                    break;
                }
            }
            Some(AnsiSegment::EscapeSequence(escape_seq))
        } else {
            Some(AnsiSegment::VisibleChar(ch))
        }
    }
}

enum AnsiSegment {
    EscapeSequence(String),
    VisibleChar(char),
}

fn calculate_visible_length(text: &str) -> usize {
    let mut parser = AnsiParser::new(text);
    let mut len = 0;

    while let Some(segment) = parser.next_segment() {
        if matches!(segment, AnsiSegment::VisibleChar(_)) {
            len += 1;
        }
    }

    len
}

fn truncate_ansi_string(text: &str, overflow_str: &str, max_len: usize) -> String {
    let visible_len = calculate_visible_length(text);
    let overflow_len = overflow_str.len();

    if visible_len <= max_len {
        return text.to_string();
    }

    if max_len <= overflow_len {
        return overflow_str.to_string();
    }

    let target_len = max_len - overflow_len;
    let mut result = String::new();
    let mut visible_count = 0;
    let mut parser = AnsiParser::new(text);
    let mut add_printable_chrs = true;

    while let Some(segment) = parser.next_segment() {
        match segment {
            AnsiSegment::EscapeSequence(seq) => {
                result.push_str(&seq);
            }
            AnsiSegment::VisibleChar(ch) => {
                if add_printable_chrs {
                    result.push(ch);
                    visible_count += 1;

                    add_printable_chrs = visible_count < target_len;
                }
            }
        }
    }

    result.push_str(overflow_str);
    result
}

// ---------------------------------------------------------------------------
// Key-finding helpers
// ---------------------------------------------------------------------------

fn find_keys_for_actions(
    keymap: &[(KeyWithModifier, Vec<Action>)],
    target_actions: &[Action],
    exact_match: bool,
) -> Vec<KeyWithModifier> {
    keymap
        .iter()
        .filter_map(|(key, key_actions)| {
            if exact_match {
                let matching = key_actions
                    .iter()
                    .zip(target_actions)
                    .filter(|(a, b)| a.shallow_eq(b))
                    .count();
                if matching == key_actions.len() && matching == target_actions.len() {
                    Some(key.clone())
                } else {
                    None
                }
            } else if key_actions.iter().next() == target_actions.iter().next() {
                Some(key.clone())
            } else {
                None
            }
        })
        .collect()
}

fn find_keys_for_action_groups(
    keymap: &[(KeyWithModifier, Vec<Action>)],
    action_groups: &[&[Action]],
) -> Vec<KeyWithModifier> {
    action_groups
        .iter()
        .flat_map(|actions| find_keys_for_actions(keymap, actions, true))
        .collect()
}

// ---------------------------------------------------------------------------
// Modifier rendering
// ---------------------------------------------------------------------------

fn modifier_name(modifier: &KeyModifier, style: ModifierStyle) -> String {
    match style {
        ModifierStyle::Long => modifier.to_string(),
        ModifierStyle::Short => match modifier {
            KeyModifier::Ctrl => "C".to_string(),
            KeyModifier::Alt => "A".to_string(),
            KeyModifier::Shift => "S".to_string(),
            _ => modifier.to_string(),
        },
        ModifierStyle::Symbol => match modifier {
            KeyModifier::Ctrl => "^".to_string(),
            KeyModifier::Alt => "M-".to_string(),
            KeyModifier::Shift => "S-".to_string(),
            _ => modifier.to_string(),
        },
    }
}

fn format_modifier_string(modifiers: &[KeyModifier], style: ModifierStyle) -> String {
    if modifiers.is_empty() {
        String::new()
    } else {
        modifiers
            .iter()
            .map(|m| modifier_name(m, style))
            .collect::<Vec<_>>()
            .join("-")
    }
}

fn modifier_separator(style: ModifierStyle) -> &'static str {
    match style {
        ModifierStyle::Long => " + ",
        ModifierStyle::Short => "-",
        ModifierStyle::Symbol => "",
    }
}

// ---------------------------------------------------------------------------
// Key display formatting
// ---------------------------------------------------------------------------

/// Return the display string for a bare key, applying `key_aliases` when present.
/// The alias map uses lowercase key names as keys (e.g. `"enter"`, `"space"`).
fn bare_key_display(bare_key: &BareKey, key_aliases: &HashMap<String, String>) -> String {
    let default = format!("{}", bare_key);
    // Look up by lowercased default representation.
    key_aliases
        .get(&default.to_lowercase())
        .cloned()
        .unwrap_or(default)
}

fn format_key_display(
    key_bindings: &[KeyWithModifier],
    common_modifiers: &[KeyModifier],
    key_aliases: &HashMap<String, String>,
) -> Vec<String> {
    key_bindings
        .iter()
        .map(|key| {
            if common_modifiers.is_empty() {
                // Full key with modifiers already rendered by Display; we only
                // substitute the bare-key portion so modifier prefixes are kept.
                let bare = bare_key_display(&key.bare_key, key_aliases);
                if key.key_modifiers.is_empty() {
                    bare
                } else {
                    let mods = key
                        .key_modifiers
                        .iter()
                        .map(|m| m.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{} {}", mods, bare)
                }
            } else {
                let unique_modifiers = key
                    .key_modifiers
                    .iter()
                    .filter(|m| !common_modifiers.contains(m))
                    .map(|m| m.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                let bare = bare_key_display(&key.bare_key, key_aliases);
                if unique_modifiers.is_empty() {
                    bare
                } else {
                    format!("{} {}", unique_modifiers, bare)
                }
            }
        })
        .collect()
}

fn get_key_separator(key_display: &[String]) -> &'static str {
    let key_string = key_display.join("");
    if KEY_PATTERNS_NO_SEPARATOR.contains(&&key_string[..]) {
        ""
    } else {
        "|"
    }
}

// ---------------------------------------------------------------------------
// Styled rendering
// ---------------------------------------------------------------------------


/// Plain-text key representation used by custom `hint_format` templates.
fn format_key_plain(
    key_bindings: &[KeyWithModifier],
    modifier_style: ModifierStyle,
    key_aliases: &HashMap<String, String>,
) -> String {
    if key_bindings.is_empty() {
        return String::new();
    }
    let common_modifiers = get_common_modifiers(key_bindings.iter().collect());
    let modifier_str = format_modifier_string(&common_modifiers, modifier_style);
    let key_display = format_key_display(key_bindings, &common_modifiers, key_aliases);
    let key_separator = get_key_separator(&key_display);
    let keys_str = key_display.join(key_separator);
    if modifier_str.is_empty() {
        keys_str
    } else {
        let sep = modifier_separator(modifier_style);
        format!("{}{}{}", modifier_str, sep, keys_str)
    }
}

fn plugin_key(
    keymap: &[(KeyWithModifier, Vec<Action>)],
    plugin_name: &str,
) -> Option<KeyWithModifier> {
    keymap.iter().find_map(|(key, key_actions)| {
        if key_actions
            .iter()
            .any(|action| action.launches_plugin(plugin_name))
        {
            Some(key.clone())
        } else {
            None
        }
    })
}

fn get_select_key(keymap: &[(KeyWithModifier, Vec<Action>)]) -> Vec<KeyWithModifier> {
    let to_normal_keys = find_keys_for_actions(keymap, &[TO_NORMAL], true);
    if to_normal_keys.contains(&KeyWithModifier::new(BareKey::Enter)) {
        vec![KeyWithModifier::new(BareKey::Enter)]
    } else {
        to_normal_keys.into_iter().take(1).collect()
    }
}

// ---------------------------------------------------------------------------
// zjstatus-style inline format parser: #[fg=#RRGGBB,bg=red,bold,italic,...]
//
// Spec follows zjstatus: each directive replaces the active style (no inherit
// across directives). Recognised attrs:
//   color: fg / bg
//   color value: #RRGGBB hex, 0..255 ANSI index, named (red, bright_blue, ...)
//   flags: bold, italic / italics, underscore, double-underscore,
//          curly-underscore, dotted-underscore, dashed-underscore,
//          blink, hidden, reverse, strikethrough
// ---------------------------------------------------------------------------

fn parse_color_token(s: &str) -> Option<Colour> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(RGB(r, g, b));
        }
        return None;
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Fixed(n));
    }
    let named = match s.to_ascii_lowercase().as_str() {
        "black" => Some(Fixed(0)),
        "red" => Some(Fixed(1)),
        "green" => Some(Fixed(2)),
        "yellow" => Some(Fixed(3)),
        "blue" => Some(Fixed(4)),
        "magenta" | "purple" => Some(Fixed(5)),
        "cyan" => Some(Fixed(6)),
        "white" => Some(Fixed(7)),
        "bright_black" | "gray" | "grey" => Some(Fixed(8)),
        "bright_red" => Some(Fixed(9)),
        "bright_green" => Some(Fixed(10)),
        "bright_yellow" => Some(Fixed(11)),
        "bright_blue" => Some(Fixed(12)),
        "bright_magenta" | "bright_purple" => Some(Fixed(13)),
        "bright_cyan" => Some(Fixed(14)),
        "bright_white" => Some(Fixed(15)),
        _ => None,
    };
    named
}

/// Parse one zjstatus-style directive body (the part inside `#[...]`).
/// Ignores unknown tokens; bad color values silently fall through.
fn parse_inline_directive(body: &str) -> Style {
    let mut style = Style::new();
    for raw in body.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(rest) = token.strip_prefix("fg=") {
            if let Some(c) = parse_color_token(rest) {
                style = style.fg(c);
            }
            continue;
        }
        if let Some(rest) = token.strip_prefix("bg=") {
            if let Some(c) = parse_color_token(rest) {
                style = style.on(c);
            }
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "bold" => style = style.bold(),
            "italic" | "italics" => style = style.italic(),
            "underscore" | "underline" => style = style.underline(),
            "double-underscore" | "curly-underscore" | "dotted-underscore"
            | "dashed-underscore" => style = style.underline(), // ansi_term has only one underline kind
            "blink" => style = style.blink(),
            "hidden" => style = style.hidden(),
            "reverse" => style = style.reverse(),
            "strikethrough" => style = style.strikethrough(),
            "dim" => style = style.dimmed(),
            _ => {}
        }
    }
    style
}

/// Render a `hint_format` using zjstatus segment-split semantics:
/// the format string is split on `#[`; each segment looks like
/// `directive_body]content` and renders `content` with the style parsed from
/// `directive_body`. Each new `#[...]` resets the style — directives do NOT
/// inherit. Inside `content`, `{key}` and `{action}` are substituted.
///
/// The first segment (before any `#[`) is rendered unstyled (terminal default).
fn render_hint_format_inline(
    parts: &mut Vec<ANSIString<'static>>,
    hint_format: &str,
    key_plain: &str,
    action_text: &str,
) {
    let leads_with_directive = hint_format.starts_with("#[");
    for (idx, segment) in hint_format.split("#[").enumerate() {
        if segment.is_empty() {
            continue;
        }
        let (style, content) = if idx == 0 && !leads_with_directive {
            (Style::new(), segment.to_string())
        } else {
            // Segment format: <body>]<content>. If no `]`, treat whole thing
            // as content with default style (matches zjstatus tolerance).
            match segment.find(']') {
                Some(end) => {
                    let body = &segment[..end];
                    let content = &segment[end + 1..];
                    (parse_inline_directive(body), content.to_string())
                }
                None => (Style::new(), segment.to_string()),
            }
        };
        let expanded = content
            .replace("{key}", key_plain)
            .replace("{action}", action_text);
        if !expanded.is_empty() {
            parts.push(style.paint(expanded));
        }
    }
}

// ---------------------------------------------------------------------------
// Hint assembly
// ---------------------------------------------------------------------------

fn add_hint(
    parts: &mut Vec<ANSIString<'static>>,
    keys: &[KeyWithModifier],
    description: &str,
    modifier_style: ModifierStyle,
    hint_format: &str,
    separator: &str,
    max_keys: usize,
    action_aliases: &HashMap<String, String>,
    key_aliases: &HashMap<String, String>,
) {
    if keys.is_empty() {
        return;
    }

    let keys = if max_keys > 0 && keys.len() > max_keys {
        &keys[..max_keys]
    } else {
        keys
    };

    let display_label: &str = action_aliases
        .get(description)
        .map(|s| s.as_str())
        .unwrap_or(description);

    if !separator.is_empty() && !parts.is_empty() {
        parts.push(Style::new().paint(separator.to_string()));
    }

    let key_plain = format_key_plain(keys, modifier_style, key_aliases);
    render_hint_format_inline(parts, hint_format, &key_plain, display_label);
}

// ---------------------------------------------------------------------------
// Per-mode hint rendering
// ---------------------------------------------------------------------------

fn render_hints_for_mode(
    mode: InputMode,
    keymap: &[(KeyWithModifier, Vec<Action>)],
    modifier_style: ModifierStyle,
    hint_format: &str,
    separator: &str,
    max_keys: usize,
    action_aliases: &HashMap<String, String>,
    key_aliases: &HashMap<String, String>,
) -> Vec<ANSIString<'static>> {
    let mut parts = vec![];
    let select_keys = get_select_key(keymap);

    macro_rules! hint {
        ($keys:expr, $label:expr) => {
            add_hint(
                &mut parts,
                $keys,
                $label,
                modifier_style,
                hint_format,
                separator,
                max_keys,
                action_aliases,
                key_aliases,
            )
        };
    }

    match mode {
        InputMode::Normal => {
            for (action, label) in NORMAL_MODE_ACTIONS {
                let keys = find_keys_for_actions(keymap, &[action.clone()], true);
                hint!(&keys, label);
            }
        }
        InputMode::Pane => {
            for (actions, label) in PANE_MODE_ACTION_SEQUENCES {
                let keys = find_keys_for_actions(keymap, actions, true);
                hint!(&keys, label);
            }

            let focus_keys_full = find_keys_for_action_groups(
                keymap,
                &[
                    &[Action::MoveFocusOrTab { direction: Direction::Left }],
                    &[Action::MoveFocusOrTab { direction: Direction::Right }],
                    &[Action::MoveFocus { direction: Direction::Left }],
                    &[Action::MoveFocus { direction: Direction::Down }],
                    &[Action::MoveFocus { direction: Direction::Up }],
                    &[Action::MoveFocus { direction: Direction::Right }],
                ],
            );
            let focus_keys = if focus_keys_full.contains(&KeyWithModifier::new(BareKey::Left))
                && focus_keys_full.contains(&KeyWithModifier::new(BareKey::Right))
            {
                vec![
                    KeyWithModifier::new(BareKey::Left),
                    KeyWithModifier::new(BareKey::Right),
                ]
            } else {
                focus_keys_full
            };
            hint!(&focus_keys, "move");
            let rename_keys = find_keys_for_actions(
                keymap,
                &[
                    Action::SwitchToMode { input_mode: InputMode::RenamePane },
                    Action::PaneNameInput { input: vec![0] },
                ],
                false,
            );
            if !rename_keys.is_empty() {
                hint!(&rename_keys, "rename");
            }
            hint!(&select_keys, "select");
        }
        InputMode::Tab => {
            for (actions, label) in TAB_MODE_ACTION_SEQUENCES {
                let keys = find_keys_for_actions(keymap, actions, true);
                hint!(&keys, label);
            }

            let focus_keys_full = find_keys_for_action_groups(
                keymap,
                &[&[Action::GoToPreviousTab], &[Action::GoToNextTab]],
            );
            let focus_keys = if focus_keys_full.contains(&KeyWithModifier::new(BareKey::Left))
                && focus_keys_full.contains(&KeyWithModifier::new(BareKey::Right))
            {
                vec![
                    KeyWithModifier::new(BareKey::Left),
                    KeyWithModifier::new(BareKey::Right),
                ]
            } else {
                focus_keys_full
            };
            hint!(&focus_keys, "move");
            let rename_keys = find_keys_for_actions(
                keymap,
                &[
                    Action::SwitchToMode { input_mode: InputMode::RenameTab },
                    Action::TabNameInput { input: vec![0] },
                ],
                false,
            );
            if !rename_keys.is_empty() {
                hint!(&rename_keys, "rename");
            }
            hint!(&select_keys, "select");
        }
        InputMode::Resize => {
            let resize_keys = find_keys_for_action_groups(
                keymap,
                &[
                    &[Action::Resize { resize: Resize::Increase, direction: None }],
                    &[Action::Resize { resize: Resize::Decrease, direction: None }],
                ],
            );
            hint!(&resize_keys, "resize");

            let increase_keys = find_keys_for_action_groups(
                keymap,
                &[
                    &[Action::Resize { resize: Resize::Increase, direction: Some(Direction::Left) }],
                    &[Action::Resize { resize: Resize::Increase, direction: Some(Direction::Down) }],
                    &[Action::Resize { resize: Resize::Increase, direction: Some(Direction::Up) }],
                    &[Action::Resize { resize: Resize::Increase, direction: Some(Direction::Right) }],
                ],
            );
            hint!(&increase_keys, "increase");

            let decrease_keys = find_keys_for_action_groups(
                keymap,
                &[
                    &[Action::Resize { resize: Resize::Decrease, direction: Some(Direction::Left) }],
                    &[Action::Resize { resize: Resize::Decrease, direction: Some(Direction::Down) }],
                    &[Action::Resize { resize: Resize::Decrease, direction: Some(Direction::Up) }],
                    &[Action::Resize { resize: Resize::Decrease, direction: Some(Direction::Right) }],
                ],
            );
            hint!(&decrease_keys, "decrease");
            hint!(&select_keys, "select");
        }
        InputMode::Move => {
            let move_keys = find_keys_for_action_groups(
                keymap,
                &[
                    &[Action::MovePane { direction: Some(Direction::Left) }],
                    &[Action::MovePane { direction: Some(Direction::Down) }],
                    &[Action::MovePane { direction: Some(Direction::Up) }],
                    &[Action::MovePane { direction: Some(Direction::Right) }],
                ],
            );
            hint!(&move_keys, "move");
            hint!(&select_keys, "select");
        }
        InputMode::Scroll => {
            let search_keys = find_keys_for_actions(
                keymap,
                &[
                    Action::SwitchToMode { input_mode: InputMode::EnterSearch },
                    Action::SearchInput { input: vec![0] },
                ],
                true,
            );
            hint!(&search_keys, "search");

            let scroll_keys =
                find_keys_for_action_groups(keymap, &[&[Action::ScrollDown], &[Action::ScrollUp]]);
            hint!(&scroll_keys, "scroll");

            let page_scroll_keys = find_keys_for_action_groups(
                keymap,
                &[&[Action::PageScrollDown], &[Action::PageScrollUp]],
            );
            hint!(&page_scroll_keys, "page");

            let half_page_scroll_keys = find_keys_for_action_groups(
                keymap,
                &[&[Action::HalfPageScrollDown], &[Action::HalfPageScrollUp]],
            );
            hint!(&half_page_scroll_keys, "half page");

            let edit_keys =
                find_keys_for_actions(keymap, &[Action::EditScrollback { ansi: false }, TO_NORMAL], false);
            if !edit_keys.is_empty() {
                hint!(&edit_keys, "edit");
            }
            hint!(&select_keys, "select");
        }
        InputMode::Search => {
            let search_keys = find_keys_for_actions(
                keymap,
                &[
                    Action::SwitchToMode { input_mode: InputMode::EnterSearch },
                    Action::SearchInput { input: vec![0] },
                ],
                true,
            );
            hint!(&search_keys, "search");

            let scroll_keys =
                find_keys_for_action_groups(keymap, &[&[Action::ScrollDown], &[Action::ScrollUp]]);
            hint!(&scroll_keys, "scroll");

            let page_scroll_keys = find_keys_for_action_groups(
                keymap,
                &[&[Action::PageScrollDown], &[Action::PageScrollUp]],
            );
            hint!(&page_scroll_keys, "page");

            let half_page_scroll_keys = find_keys_for_action_groups(
                keymap,
                &[&[Action::HalfPageScrollDown], &[Action::HalfPageScrollUp]],
            );
            hint!(&half_page_scroll_keys, "half page");

            let down_keys =
                find_keys_for_actions(keymap, &[Action::Search { direction: SearchDirection::Down }], true);
            hint!(&down_keys, "down");

            let up_keys =
                find_keys_for_actions(keymap, &[Action::Search { direction: SearchDirection::Up }], true);
            hint!(&up_keys, "up");

            hint!(&select_keys, "select");
        }
        InputMode::Session => {
            let detach_keys = find_keys_for_actions(keymap, &[Action::Detach], true);
            hint!(&detach_keys, "detach");

            if let Some(manager_key) = plugin_key(keymap, PLUGIN_SESSION_MANAGER) {
                hint!(&[manager_key], "manager");
            }

            if let Some(config_key) = plugin_key(keymap, PLUGIN_CONFIGURATION) {
                hint!(&[config_key], "config");
            }

            if let Some(plugin_key_val) = plugin_key(keymap, PLUGIN_MANAGER) {
                hint!(&[plugin_key_val], "plugins");
            }

            if let Some(about_key) = plugin_key(keymap, PLUGIN_ABOUT) {
                hint!(&[about_key], "about");
            }

            hint!(&select_keys, "select");
        }
        _ => {
            let keys =
                find_keys_for_actions(keymap, &[Action::SwitchToMode { input_mode: InputMode::Normal }], true);
            hint!(&keys, "normal");
        }
    }

    parts
}

fn get_keymap_for_mode(mode_info: &ModeInfo) -> Vec<(KeyWithModifier, Vec<Action>)> {
    match mode_info.mode {
        InputMode::Normal => mode_info.get_keybinds_for_mode(InputMode::Normal),
        InputMode::Pane => mode_info.get_keybinds_for_mode(InputMode::Pane),
        InputMode::Tab => mode_info.get_keybinds_for_mode(InputMode::Tab),
        InputMode::Resize => mode_info.get_keybinds_for_mode(InputMode::Resize),
        InputMode::Move => mode_info.get_keybinds_for_mode(InputMode::Move),
        InputMode::Scroll => mode_info.get_keybinds_for_mode(InputMode::Scroll),
        InputMode::Search => mode_info.get_keybinds_for_mode(InputMode::Search),
        InputMode::Session => mode_info.get_keybinds_for_mode(InputMode::Session),
        _ => mode_info.get_mode_keybinds(),
    }
}
