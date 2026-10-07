# 鼠标支持：共享布局模型的命中区 + 捕获取舍与逃逸

鼠标经一处纯函数「布局模型」接入：该函数由 `(area, App, 尺寸档)` 算出所有可交互矩形的**命中区**清单，渲染与鼠标命中测试调用同一份——渲染保持无副作用、命中区可在 `TestBackend` 下单测。V1 交互范围 = 点导航切屏 / 点列表行选中 / 滚轮移选中或在装配 stderr 日志区滚动（为此新增 `as_log_offset`）/ 发射仪式等浮层按钮可点；浮层优先级对鼠标同样成立。鼠标捕获默认开启，代价是终端「拖拽选中、右键复制」需按 Shift；提供逃逸开关把鼠标释放回终端。

Status: accepted

## Considered Options

- **渲染时把 Rect 记进 clickmap（`&mut App` / `RefCell`）**：排除。破坏「渲染无副作用」的纪律且难测；同一批 Rect 要渲染与输入都说真话，纯函数才是单一真相来源。
- **依赖 ratatui 内置热区**：ratatui 0.30 无原生可点击 widget，无此物可用。

## Consequences

- `term::init()` 增补 `EnableMouseCapture`，`restore()` 对应 `DisableMouseCapture`；主循环把今天被丢弃的 `Event::Mouse` 接入 `Message`（见 main.rs 输入分派）。`Event::Resize` 无需专门处理——尺寸档每帧由 `area` 现算，跨断点 resize 自然换态。
- 逃逸开关两联：`--no-mouse` CLI 旗标 + 设置屏开关（持久化进 config）；运行时热键砍掉，省去活动终端重配捕获的路径与额外键位。
- V1 滚轮滚动只覆盖装配 stderr 日志区（为此新增 `as_log_offset`）；概览「最近日志」、健康「证据」维持现有截断，滚动留待后续。
- 键盘流程完全不变，鼠标是纯增量，不引入任何键盘回归。
