use crate::capture::claude::detection;
use crate::capture::CaptureError;
use crate::models::{
    AccessibilityNode, AccessibilitySnapshot, Bounds, DetectedProcess, DetectedWindow,
    InspectorOptions,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};
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
const MIN_TOP_LEVEL_WIDTH: i32 = 320;
const MIN_TOP_LEVEL_HEIGHT: i32 = 240;
const UIA_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
const UIA_TRAVERSAL_BUDGET: Duration = Duration::from_secs(8);

struct TraversalState {
    max_depth: usize,
    max_elements: usize,
    next_id: usize,
    element_count: usize,
    truncated: bool,
    deadline: Instant,
}

pub fn find_claude_windows(
    processes: &[DetectedProcess],
) -> Result<Vec<DetectedWindow>, CaptureError> {
    let mut windows = enumerate_top_level_windows(processes)
        .into_iter()
        .filter(|window| !window.detection_signals.is_empty())
        .collect::<Vec<_>>();
    windows.sort_by(|left, right| window_rank(right).cmp(&window_rank(left)));
    Ok(windows)
}

pub fn inspect_first_claude_window(
    processes: &[DetectedProcess],
    options: InspectorOptions,
) -> Result<AccessibilitySnapshot, CaptureError> {
    let processes = processes.to_vec();
    let timeout_options = options.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("claude-uia-inspector".to_string())
        .spawn(move || {
            let _ = sender.send(inspect_first_claude_window_on_uia_thread(
                &processes, options,
            ));
        })
        .map_err(|error| {
            CaptureError::Native(format!("Failed to start UI Automation thread: {error}"))
        })?;

    match receiver.recv_timeout(UIA_COMMAND_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(empty_snapshot(
            timeout_options,
            "UI Automation inspection timed out while reading the detected Claude Desktop window. Phase 2 finding: Claude was detected by process and HWND, but this window did not expose a readable UIA root before the timeout.",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(CaptureError::Native(
            "UI Automation thread stopped before returning a snapshot.".to_string(),
        )),
    }
}

fn inspect_first_claude_window_on_uia_thread(
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
    let target_summary = target_window_summary(&window);
    let claude_root = match automation.element_from_handle(Handle::from(hwnd)) {
        Ok(element) => element,
        Err(error) => {
            return Ok(empty_snapshot(
                options,
                &format!("UI Automation could not inspect {target_summary}: {error}"),
            ));
        }
    };

    let max_depth = options.max_depth.unwrap_or(DEFAULT_MAX_DEPTH).clamp(0, 30);
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
        deadline: Instant::now() + UIA_TRAVERSAL_BUDGET,
    };

    let root_name = get_name(&claude_root);
    let root_node = collect_node(&claude_root, &walker, 0, &mut state);
    let mut warnings = vec![format!("Inspected {target_summary}.")];
    if state.truncated {
        warnings.push(format!(
            "Snapshot stopped at {} elements, depth {}, or the {} second traversal budget to keep diagnostics responsive.",
            state.max_elements,
            state.max_depth,
            UIA_TRAVERSAL_BUDGET.as_secs()
        ));
    }
    if root_node.is_none() {
        warnings.push(
            "UI Automation returned a root handle, but no tree nodes could be read.".to_string(),
        );
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

fn target_window_summary(window: &DetectedWindow) -> String {
    format!(
        "window '{}' ({}, {}, {})",
        if window.title.is_empty() {
            "Untitled"
        } else {
            &window.title
        },
        window.hwnd.as_deref().unwrap_or("unknown HWND"),
        window.class_name.as_deref().unwrap_or("unknown class"),
        window
            .bounds
            .map(|bounds| format!(
                "{}x{} at {},{}",
                bounds.width, bounds.height, bounds.x, bounds.y
            ))
            .unwrap_or_else(|| "unknown bounds".to_string())
    )
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
        max_depth: options.max_depth.unwrap_or(DEFAULT_MAX_DEPTH).clamp(0, 30),
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

    if is_usable_desktop_surface(&window)
        && !detection::is_known_non_desktop_window(&window.title, &window.class_name)
    {
        if detection::is_likely_claude_window_title(&window.title) {
            signals.push("window-title".to_string());
        }
        if process.is_some() {
            signals.push("process-id".to_string());
        }
        if process
            .map(|process| {
                detection::is_likely_claude_desktop_process(&process.name, process.path.as_deref())
            })
            .unwrap_or(false)
        {
            signals.push("process-name".to_string());
        }
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

fn is_usable_desktop_surface(window: &RawWindow) -> bool {
    window.visible
        && window
            .bounds
            .map(|bounds| {
                bounds.width >= MIN_TOP_LEVEL_WIDTH && bounds.height >= MIN_TOP_LEVEL_HEIGHT
            })
            .unwrap_or(false)
}

fn window_rank(window: &DetectedWindow) -> i32 {
    let mut score = 0;
    if window
        .detection_signals
        .iter()
        .any(|signal| signal == "window-title")
    {
        score += 100;
    }
    if window
        .detection_signals
        .iter()
        .any(|signal| signal == "process-name")
    {
        score += 40;
    }
    if window
        .detection_signals
        .iter()
        .any(|signal| signal == "process-id")
    {
        score += 20;
    }
    if window
        .class_name
        .as_deref()
        .map(|class_name| class_name.contains("Chrome_WidgetWin"))
        .unwrap_or(false)
    {
        score += 15;
    }
    if window.visible {
        score += 10;
    }
    if let Some(bounds) = window.bounds {
        score += ((bounds.width * bounds.height) / 100_000).clamp(0, 20);
    }
    score
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
    if Instant::now() >= state.deadline {
        state.truncated = true;
        return None;
    }

    if state.element_count >= state.max_elements {
        state.truncated = true;
        return None;
    }

    let id = state.next_id;
    state.next_id += 1;
    state.element_count += 1;

    let control_type = element
        .get_control_type()
        .ok()
        .map(|control_type| format!("{control_type:?}"));
    let localized_control_type = get_localized_control_type(element);
    let name = get_name(element);
    let automation_id = get_automation_id(element);
    let class_name = get_classname(element);
    let framework_id = get_framework_id(element);
    let bounds = element.get_bounding_rectangle().ok().map(|rect| Bounds {
        x: rect.get_left(),
        y: rect.get_top(),
        width: rect.get_width(),
        height: rect.get_height(),
    });
    let enabled = element.is_enabled().ok();
    let has_keyboard_focus = element.has_keyboard_focus().ok();
    let offscreen = element.is_offscreen().ok();
    let supported_patterns = supported_patterns(element);

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
    let value = if depth > 0 && child_count == 0 {
        get_value(element)
    } else {
        None
    };

    Some(AccessibilityNode {
        id,
        depth,
        control_type,
        localized_control_type,
        name,
        automation_id,
        class_name,
        framework_id,
        value,
        bounds,
        enabled,
        has_keyboard_focus,
        offscreen,
        supported_patterns,
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
