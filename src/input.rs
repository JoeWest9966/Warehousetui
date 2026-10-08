//! 通用单行输入态机(#26 预重构):「要一个字符串参数」的流程共用的键入件。
//!
//! 从设置屏的 NW_HOME 手填流程原样剥出(纯重构,零用户可见行为变化):
//! 本模块零 I/O、零副作用、不认识任何字段语义——按键分发在此收口,
//! 提交的**校验权归消费者**(拒绝 → 留驻可改,拒绝本身不留痕迹)。
//! 既定语义与重构前逐条一致:Enter 请求提交、Esc/q 取消、退格删字符、
//! 只收可打印字符(控制字符与 CONTROL 修饰一律忽略)、缓冲封顶。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 缓冲上限(防误粘贴;按字符计)。
pub const BUF_CAP: usize = 512;

/// 一次按键的分派结果;后果链(日志/状态/持久化)归消费者。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputAction {
    /// Enter:请求提交;校验不过时消费者留在输入态可改。
    Submit,
    /// Esc / q:取消(消费者清缓冲、退输入态)。
    Cancel,
    /// 缓冲有变动(入栈或退格)。
    Edited,
    /// 其余按键或被拒字符:忽略。
    Ignored,
}

/// 按键分发:维护 `buf` 并返回动作。分支顺序与重构前的 match 一致
/// (Enter → Esc/q → 退格 → 字符),保证语义零漂移。
pub fn on_key(buf: &mut String, k: KeyEvent) -> InputAction {
    match k.code {
        KeyCode::Enter => InputAction::Submit,
        KeyCode::Esc | KeyCode::Char('q') => InputAction::Cancel,
        KeyCode::Backspace => {
            buf.pop();
            InputAction::Edited
        }
        KeyCode::Char(c) => {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            if !ctrl && !c.is_control() && buf.chars().count() < BUF_CAP {
                buf.push(c);
                InputAction::Edited
            } else {
                InputAction::Ignored
            }
        }
        _ => InputAction::Ignored,
    }
}

/// 提交前规范化:去首尾空白;空/纯空白 → `None`(拒绝,消费者留驻)。
pub fn normalize(buf: &str) -> Option<String> {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn 应用_enter请求提交且不动缓冲() {
        let mut buf = String::from("/data");
        let a = on_key(&mut buf, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, InputAction::Submit, "Enter 请求提交");
        assert_eq!(buf, "/data", "分发本身不改缓冲,后果归消费者");
        assert_eq!(
            normalize("  /data/x  "),
            Some("/data/x".into()),
            "规范化去空白"
        );
    }

    #[test]
    fn 取消_esc与_q同路() {
        let mut buf = String::from("x");
        assert_eq!(
            on_key(&mut buf, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InputAction::Cancel
        );
        assert_eq!(
            on_key(&mut buf, press('q')),
            InputAction::Cancel,
            "q 与 Esc 同义"
        );
        assert_eq!(buf, "x", "取消分发不碰缓冲(清空归消费者)");
    }

    #[test]
    fn 拒绝留驻_纯空白提交被拒() {
        assert_eq!(normalize(""), None, "空值拒绝,消费者留驻可改");
        assert_eq!(normalize("   "), None, "纯空白同样拒绝");
        assert_eq!(normalize(" a "), Some("a".into()), "有内容才放行");
    }

    #[test]
    fn 上限_512封顶_可退格删改() {
        let mut buf = String::new();
        for _ in 0..(BUF_CAP + 88) {
            on_key(&mut buf, press('a'));
        }
        assert_eq!(buf.chars().count(), BUF_CAP, "上限防误粘贴");
        on_key(
            &mut buf,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(buf.chars().count(), BUF_CAP - 1, "封顶后仍可退格删改");
    }
}
