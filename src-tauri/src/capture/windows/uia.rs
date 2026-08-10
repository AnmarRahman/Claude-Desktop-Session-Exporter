use crate::capture::claude::detection;
use crate::capture::CaptureError;
use crate::models::{
    AccessibilityNode, AccessibilitySnapshot, Bounds, DetectedProcess, DetectedWindow,
    InspectorOptions,
};
use uiautomation::patterns::{UITextPattern, UIValuePattern};
use uiautomation::types::{Handle, UIProperty};
use uiautomation::{UIAutomation, UIElement, UITreeWalker};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible,
};
use windows_result::BOOL;

const DEFAULT_MAX_DEPTH: usize = 12;
const DEFAULT_MAX_ELEMENTS: usize = 5_000;
const MAX_TEXT_PATTERN_CHARS: i32 = 20_000;

struct TraversalState {
    max_depth: usize,
    max_elements: usize,
    next_id: usize,
    element_count: usize,
    truncated: bool,
}

pub fn find_claude_windows(
    processes: &[DetectedProcess],
) -> Result<Vec<DetectedWindow>, CaptureError> {
    Ok(enumerate_top_level_windows(processes)
        .into_iter()
        .filter(|window| !window.detection_signals.is_empty())
        .collect())
}

pub fn inspect_first_claude_window(
    processes: &[DetectedProcess],
    options: InspectorOptions,
) -> Result<AccessibilitySnapshot, CaptureError> {
    let automation = UIAutomation::new().map_err(native_error)?;
    let walker = tree_walker(&automation, options.tree_view.as_deref())?;
    let windows = find_claude_windows(processes)?;
    let Some(window) = windows
        .into_iter()
        .find(|window| window.visible)
        .or_else(|| find_claude_windows(processes).ok()?.into_iter().next())
    else {
        return Ok(empty_snapshot(
            options,
            "Claude Desktop top-level HWND was not found.",
        ));
    };
    let hwnd =
        window.hwnd.as_deref().and_then(parse_hwnd).ok_or_else(|| {
            CaptureError::Native("Claude HWND was missing or invalid.".to_string())
        })?;
    let claude_root = automation
        .element_from_handle(Handle::from(hwnd))
        .map_err(native_error)?;

    let max_depth = options.max_depth.unwrap_or(DEFAULT_MAX_DEPTH).clamp(1, 30);
    let max_elements = options
        .max_elements
        .unwrap_or(DEFAULT_MAX_ELEMENTS)
        .clamp(1, 25_000);
    let mut state = TraversalState {
        max_depth,
        max_elements,
        next_id: 1,
        element_count: 0,
        truncated: false,
    };

    let root_name = get_name(&claude_root);
    let root_node = collect_node(&claude_root, &walker, 0, &mut state);
    let mut warnings = Vec::new();
    if state.truncated {
        warnings.push(format!(
            "Snapshot stopped at {} elements or depth {} to keep diagnostics responsive.",
            state.max_elements, state.max_depth
        ));
    }

    Ok(AccessibilitySnapshot {
        platform: "windows-uiautomation".to_string(),
        root_name,
        max_depth: state.max_depth,
        max_elements: state.max_elements,
        element_count: state.element_count,
        truncated: state.truncated,
        nodes: root_node.into_iter().collect(),
        conversation_candidates: vec![],
        visible_text_blocks: vec![],
        warnings,
    })
}

fn tree_walker(
    automation: &UIAutomation,
    tree_view: Option<&str>,
) -> Result<UITreeWalker, CaptureError> {
    match tree_view.unwrap_or("control").to_ascii_lowercase().as_str() {
        "raw" => automation.get_raw_view_walker().map_err(native_error),
        "content" => automation.get_content_view_walker().map_err(native_error),
        _ => automation.get_control_view_walker().map_err(native_error),
    }
}

fn empty_snapshot(options: InspectorOptions, warning: &str) -> AccessibilitySnapshot {
    AccessibilitySnapshot {
        platform: "windows-uiautomation".to_string(),
        root_name: None,
        max_depth: options.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
        max_elements: options.max_elements.unwrap_or(DEFAULT_MAX_ELEMENTS),
        element_count: 0,
        truncated: false,
        nodes: vec![],
        conversation_candidates: vec![],
        visible_text_blocks: vec![],
        warnings: vec![warning.to_string()],
    }
}

fn enumerate_top_level_windows(processes: &[DetectedProcess]) -> Vec<DetectedWindow> {
    let mut raw_windows = Vec::<RawWindow>::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM((&mut raw_windows as *mut Vec<RawWindow>) as isize),
        );
    }

    raw_windows
        .into_iter()
        .map(|window| detected_window_from_raw(window, processes))
        .collect()
}

fn detected_window_from_raw(window: RawWindow, processes: &[DetectedProcess]) -> DetectedWindow {
    let process = processes
        .iter()
        .find(|process| Some(process.pid) == window.process_id);
    let mut signals = Vec::new();
    if detection::is_likely_claude_window_title(&window.title) {
        signals.push("window-title".to_string());
    }
    if process.is_some() {
        signals.push("process-id".to_string());
    }
    if process
        .map(|process| detection::is_likely_claude_process_name(&process.name))
        .unwrap_or(false)
    {
        signals.push("process-name".to_string());
    }

    DetectedWindow {
        title: window.title,
        process_id: window.process_id,
        process_name: process.map(|process| process.name.clone()),
        hwnd: Some(format_hwnd(window.hwnd)),
        class_name: clean_string(window.class_name),
        visible: window.visible,
        bounds: window.bounds,
        detection_signals: signals,
    }
}

#[derive(Debug)]
struct RawWindow {
    hwnd: isize,
    process_id: Option<u32>,
    title: String,
    class_name: String,
    visible: bool,
    bounds: Option<Bounds>,
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<RawWindow>);
    windows.push(RawWindow {
        hwnd: hwnd.0 as isize,
        process_id: window_process_id(hwnd),
        title: window_text(hwnd),
        class_name: window_class_name(hwnd),
        visible: IsWindowVisible(hwnd).as_bool(),
        bounds: window_bounds(hwnd),
    });
    true.into()
}

fn window_process_id(hwnd: HWND) -> Option<u32> {
    let mut process_id = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == 0 {
        None
    } else {
        Some(process_id)
    }
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; 256];
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn window_bounds(hwnd: HWND) -> Option<Bounds> {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return None;
    }

    Some(Bounds {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

fn format_hwnd(hwnd: isize) -> String {
    format!("0x{:X}", hwnd as usize)
}

fn parse_hwnd(value: &str) -> Option<isize> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed.strip_prefix("0x") {
        isize::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse().ok()
    }
}

fn collect_node(
    element: &UIElement,
    walker: &UITreeWalker,
    depth: usize,
    state: &mut TraversalState,
) -> Option<AccessibilityNode> {
    if state.element_count >= state.max_elements {
        state.truncated = true;
        return None;
    }

    let id = state.next_id;
    state.next_id += 1;
    state.element_count += 1;

    let mut children = Vec::new();
    if depth < state.max_depth {
        if let Some(raw_children) = walker.get_children(element) {
            for child in raw_children {
                if let Some(child_node) = collect_node(&child, walker, depth + 1, state) {
                    children.push(child_node);
                }
                if state.element_count >= state.max_elements {
                    break;
                }
            }
        }
    } else if walker
        .get_children(element)
        .map(|children| !children.is_empty())
        .unwrap_or(false)
    {
        state.truncated = true;
    }

    let child_count = children.len();
    Some(AccessibilityNode {
        id,
        depth,
        control_type: element
            .get_control_type()
            .ok()
            .map(|control_type| format!("{control_type:?}")),
        localized_control_type: get_localized_control_type(element),
        name: get_name(element),
        automation_id: get_automation_id(element),
        class_name: get_classname(element),
        framework_id: get_framework_id(element),
        value: get_value(element),
        bounds: element.get_bounding_rectangle().ok().map(|rect| Bounds {
            x: rect.get_left(),
            y: rect.get_top(),
            width: rect.get_width(),
            height: rect.get_height(),
        }),
        enabled: element.is_enabled().ok(),
        has_keyboard_focus: element.has_keyboard_focus().ok(),
        offscreen: element.is_offscreen().ok(),
        supported_patterns: supported_patterns(element),
        child_count,
        children,
    })
}

fn supported_patterns(element: &UIElement) -> Vec<String> {
    let candidates = [
        (UIProperty::IsInvokePatternAvailable, "Invoke"),
        (UIProperty::IsScrollPatternAvailable, "Scroll"),
        (UIProperty::IsScrollItemPatternAvailable, "ScrollItem"),
        (UIProperty::IsTextPatternAvailable, "Text"),
        (UIProperty::IsTextPattern2Available, "Text2"),
        (UIProperty::IsTextEditPatternAvailable, "TextEdit"),
        (UIProperty::IsValuePatternAvailable, "Value"),
        (UIProperty::IsGridPatternAvailable, "Grid"),
        (UIProperty::IsGridItemPatternAvailable, "GridItem"),
        (UIProperty::IsTablePatternAvailable, "Table"),
        (UIProperty::IsTableItemPatternAvailable, "TableItem"),
        (UIProperty::IsWindowPatternAvailable, "Window"),
        (UIProperty::IsSelectionPatternAvailable, "Selection"),
        (UIProperty::IsSelectionItemPatternAvailable, "SelectionItem"),
        (
            UIProperty::IsVirtualizedItemPatternAvailable,
            "VirtualizedItem",
        ),
    ];

    candidates
        .into_iter()
        .filter_map(|(property, name)| {
            property_bool(element, property)
                .filter(|available| *available)
                .map(|_| name.to_string())
        })
        .collect()
}

fn get_value(element: &UIElement) -> Option<String> {
    if let Ok(value_pattern) = element.get_pattern::<UIValuePattern>() {
        if let Ok(value) = value_pattern.get_value() {
            return clean_string(value);
        }
    }

    if let Ok(text_pattern) = element.get_pattern::<UITextPattern>() {
        if let Ok(range) = text_pattern.get_document_range() {
            if let Ok(value) = range.get_text(MAX_TEXT_PATTERN_CHARS) {
                return clean_string(value);
            }
        }
    }

    None
}

fn property_bool(element: &UIElement, property: UIProperty) -> Option<bool> {
    element
        .get_property_value(property)
        .ok()
        .and_then(|variant| variant.try_into().ok())
}

fn get_name(element: &UIElement) -> Option<String> {
    element.get_name().ok().and_then(clean_string)
}

fn get_automation_id(element: &UIElement) -> Option<String> {
    element.get_automation_id().ok().and_then(clean_string)
}

fn get_classname(element: &UIElement) -> Option<String> {
    element.get_classname().ok().and_then(clean_string)
}

fn get_framework_id(element: &UIElement) -> Option<String> {
    element.get_framework_id().ok().and_then(clean_string)
}

fn get_localized_control_type(element: &UIElement) -> Option<String> {
    element
        .get_localized_control_type()
        .ok()
        .and_then(clean_string)
}

fn clean_string(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn native_error(error: uiautomation::Error) -> CaptureError {
    CaptureError::Native(error.to_string())
}
