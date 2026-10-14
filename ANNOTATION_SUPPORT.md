# 批注与 XFDF 支持清单

更新：2026-10-14，适用于当前仓库源码；本批升级尚未发布到 crates.io。

这里区分三个层次：**数据解析/保留**、**PDF 字典读写**和**可视外观**。
支持基本往返不代表覆盖该类型的全部属性，也不代表跨阅读器像素一致。

## 1. 已有基本 XFDF ↔ PDF 通路

| XFDF 标签 | PDF 导出/回读 | 外观及主要边界 |
| --- | --- | --- |
| `text` | 基础字段、图标、打开状态 | 依赖阅读器生成便签外观 |
| `highlight`、`underline`、`strikeout`、`squiggly` | 基础字段、QuadPoints | 依赖阅读器生成标记外观 |
| `freetext` | 基础字段、DA/DS、TextColor、对齐、CL/LE/RD/BS | 文字、引出线及实线/虚线 AP；不是完整富文本排版 |
| `square`、`circle` | 基础几何、线宽、BS；Circle 填充色 | 实线/虚线和透明度 AP；其他边框样式见下文 |
| `line` | 端点、线宽、端点样式、BS | 线条/箭头 AP；测量 Intent 仅作为元数据保留 |
| `polygon`、`polyline` | 顶点、闭合语义、线宽、BS | 实线/虚线 AP；不生成云线或测量外观 |
| `ink` | 多条手势路径和线宽 | 写入 InkList，由阅读器呈现；不是完整笔刷引擎 |
| `stamp` | 图标、支持的图片数据 | 有图片 AP；外部 PDF 的复杂图章不保证完整回提 |
| `popup` | 打开状态、父注释关联 | 依赖阅读器弹窗；支持嵌套 XFDF Popup 的父子关系 |
| `link`（第一批新增） | URI、命名目的地、内部页面跳转、BS | 写入交互字典、Border、H、QuadPoints；不生成页面链接文字 |
| `caret`（第一批新增） | `/Sy`、`/RD`、基础字段 | 有插入符/段落符 AP，支持颜色、透明度和内边距 |

### Link 的具体范围

- 标准 XFDF `OnActivation/Action/URI`、`Dest/Named`、`Dest/XYZ` 等结构。
- 兼容输入 `uri="..."`、`dest="..."` 简写，输出规范化为子元素。
- 内部目的地支持 `XYZ`、`Fit`、`FitH`、`FitV`、`FitR`、`FitB`、`FitBH`、`FitBV`，保留允许的空参数。
- PDF `/Dest` 和 `/A /GoTo` 均可回读；导出在完整页面树建立后填入真实页面对象引用。
- 页码从 **0** 开始。新 PDF 页数由注释所在页决定，不会仅因链接目标页更大就自动补页；目标不存在时在保存前报错。
- 合并到已有 PDF 时，可跳转到原有页或因注释而新增的页。
- 命名目的地只保留名称引用，不复制或创建名称树。新 PDF 中未定义的名称不会因此变成可用跳转；合并时需目标 PDF 已有对应定义。
- 不实现 JavaScript、Launch、GoToR 等动作；无法处理的 XFDF 链接在兼容模式下整体保留为 XML 并警告，不转换成一个不同目标的链接。

### Caret 的具体范围

- `symbol="None"` 表示插入符标记，`symbol="P"` 表示段落标记。
- `fringe="left,bottom,right,top"` 对应 PDF `/RD` 的四边差值，不是绝对坐标。
- 有效非退化矩形且内边距留有绘制空间时生成 AP；不执行正文编辑或删除。
- 当前未做 Acrobat、PDFium、MuPDF 的跨阅读器可视一致性验收。

## 2. 第二批：Intent、Callout 与 Border/Dash

| 能力 | XFDF / PDF 映射 | 外观边界 |
| --- | --- | --- |
| Intent | 标准 `intent`，兼容旧 `IT`；读写 PDF `/IT` | 不再强制覆盖成 `FreeTextTypeWriter`；名称保留不等于执行其专用效果 |
| FreeText Callout | `callout` 的 4 或 6 个数字 ↔ `/CL`；`head` ↔ 单个 `/LE` 名称 | 直线/折线引出线、端点和文字框 AP；无内容时仍保留几何 |
| FreeText Fringe | `fringe="left,bottom,right,top"` ↔ `/RD` | 四边差值定义文字框；文字在框内裁剪，不是绝对坐标 |
| 边框样式 | `style`，兼容 `border-style`；`solid/dash/bevelled/inset/underline` ↔ `/BS /S` | 实线/虚线生成 AP；后三者只写字典、移除替代 AP 并给出转换警告，交给阅读器显示 |
| 虚线数组 | `dashes="长度,间隔,..."` ↔ `/BS /D`；兼容回读旧 `/Border` 数组 | FreeText、Square、Circle、Line、Polygon/PolyLine 的 AP 使用相同虚线；Ink/Link 由阅读器呈现 |
| 线宽与透明度 | 支持 BS 的类型读写宽度；图形/Callout AP 使用透明度状态 | 保留小于 1 的线宽；显式 0 不绘制描边，不变成 PDF hairline |

- BS 适用于 FreeText、Square、Circle、Line、Polygon、PolyLine、Ink、Link；其他类型的同名 XFDF 属性仍按扩展保留。
- Callout 的点顺序为端点、可选折点、文字框连接点；无显式 Intent 时推断 `FreeTextCallout`。XFDF 未指定线宽时 Callout 默认 1，普通 FreeText 默认 0；显式 0 会保留。手工构造 SDK 对象时需自行设置线宽。
- 引出线伸出原 `rect` 时扩大 PDF `/Rect` 和 AP `/BBox`，同步重算 `/RD`，保持文字框位置/大小；不会修改调用方对象。
- FreeText Callout 和 Line 可绘制 `None`、`Square`、`Circle`、`Diamond`、`OpenArrow`、`ClosedArrow`、`Butt`、`ROpenArrow`、`RClosedArrow`、`Slash`。这不代表 PolyLine 的端点属性也已建模。
- 只有 `dashes` 时推断虚线；`style="dash"` 未指定数组时使用 `[3]`；空数组保持空。显式 `style="solid"` 优先，数组可保留但 AP 不使用它。
- Callout 与显式非 Callout Intent 冲突会产生 Warning；不偷偷覆盖调用方意图。两个别名冲突时标准属性优先，亦报告 Warning。
- 未知边框名称、`cloudy` 和 CSS 风格 `style` 保存在 `extra` 并提示未映射，不等同于 PDF 边框实现。
- 非有限坐标、负线宽、非法虚线及端点样式会诊断；严格模式失败。Fringe 若未留下有效文字区域，PDF 导出失败且不覆盖输出。
- 文字复用现有西文/CJK 渲染；CJK 需要可用系统字体或 `PDFXML_CJK_FONT`，不是完整多行/富文本排版引擎。

已对 `examples/advanced-annotations.xfdf` 做 CLI 严格转换及 PDF→XFDF→PDF 往返，7 条注释全部保留；MuPDF 单引擎渲染前后像素一致。另以系统字体验证中文 Callout 可见及其往返渲染一致。**尚未完成 Acrobat/PDFium 等跨阅读器验收。**

## 3. XFDF 数据保真与诊断

| 能力 | 当前实现 |
| --- | --- |
| 文本与实体 | 解码属性实体，拼接 Text/CDATA；保留 `contents` 首尾空白与显式空内容 |
| 文档引用 | `f/href` 独立建模；不会自动打开引用文件 |
| 文档标识 | `ids/original`、`ids/modified` XFDF 往返；不修改 PDF trailer ID |
| 表单字段 | 嵌套 `field`、多个 `value`、空值与无值区分；**不填写 PDF AcroForm** |
| Flags | 命名标志、`0x` 十六进制、历史无前缀十六进制；输出优先使用命名标志 |
| Rich text | 保留 XML/CDATA，在 PDF 中读写 `/RC`；FreeText AP 仍采用纯文本渲染 |
| 通用扩展 | 未建模的注释属性留在 `extra`；未知子元素/文档元素保存为 XML 扩展 |
| 未实现类型 | 在 `unsupported_annotations` 中保存 XML，并返回警告；不计入可导出的 `annotations` |
| 确定性输出 | 属性按稳定顺序写出，不再强行截断浮点精度 |

保留的是内容和结构语义，不是逐字节拷贝：前缀/属性顺序、空元素形式、CDATA 写法、注释排列等可能规范化，XML 注释和处理指令不保证保留。
已识别的表单/结构元素内未建模的属性或子结构也并非全部原样保留；请检查诊断，而不要将此功能视为任意 XML 的无损编辑器。

### 兼容与严格模式

- `parse` / `load_xfdf`：默认兼容模式，记录 warning，继续处理可支持的数据。
- `parse_with_report` / `parse_with_options` / `load_xfdf_with_options`：提供结构化诊断，包括严重级别、类别、元素、字节偏移与注释名称（如有）。
- `XfdfParseMode::Strict` / CLI `--strict`：遇到 Warning 即失败；允许可保留扩展的 Info 诊断。
- **严格模式不是完整 XSD 校验，也不是 PDF 无损转换保证。** 表单、富文本、命名目的地等仍需检查 `pdf_conversion_warnings()`。
- GUI 的 XFDF→PDF 结果消息会显示解析警告和转换限制；PDF→XFDF 的跳过项目前仍主要通过日志报告。

### 安全与资源边界

- 接受 UTF-8/XML 1.0（含 UTF-8 BOM）；其他编码应由调用方先转码。
- 禁止 DTD/外部实体；不加载外部资源，不执行链接动作。
- 非法 XML、重复属性（含命名空间展开后的重复）、未闭合结构等直接报错；兼容模式也不忽略这些错误。
- 默认输入上限 64 MiB，嵌套深度 128，元素节点 250,000，注释 100,000，最大零基页索引 10,000；可通过 `XfdfParseOptions` 调整。
- 以上是 XFDF 解析限制，不是 PDF 解压/图片解码的全局资源限制。手工构造的 SDK 对象仍需调用方控制规模。
- URI 按数据保留，不验证其安全性；输出 PDF 不是经过安全净化的文件，业务系统应自行限制 URI 协议和可访问目的地。

### PDF 输入兼容（当前源码，尚未发布）

- PDF→XFDF 与合并到现有 PDF 共用 `src/pdf/reader.rs`，SDK、CLI 及调用这些 API 的 GUI 服务层均无需额外预处理。
- 首先使用原始字节加载；解析失败且最新 `startxref` 明确指向 `xref` 后紧接单个空格/制表符和首个分段编号时，只在内存中适配这个分隔符，再交给底层解析器校验。
- 不移动任何对象偏移、不全局替换内容流、不创建修复版输入文件，也不删除尾部附加数据。兼容成功会记录 warning。
- 原始 `examples/mwnodbDo.pdf` 同时包含 `xref 0 175` 与 `WebFastLoad` 尾部数据；本次阻塞原因是前者，而不是后者。原文件可直接读取和合并，两个输入文件 SHA-256 均保持不变。
- 不回退到较早的增量修订，不掩盖错误偏移/无效结构；这不是通用损坏 PDF 修复器，也不保证任意长度尾部数据或历史 `/Prev` 链中其他畸形结构均可加载。
- 合并仍输出新 PDF，不承诺保留 PDF 结构以外的厂商尾部数据或原数字签名有效性。

## 4. 尚未实现的转换能力

- `fileattachment`：仅 XFDF XML 保留，尚无 EmbeddedFiles/Filespec 导出与附件提取策略。
- `sound`：仅 XFDF XML 保留，没有声音流处理。
- `widget` / AcroForm：仅 XFDF 数据保留，没有表单控件建模、字段绑定和填写。
- `redact` 及其他未建模类型：仅 XFDF XML 保留；**没有安全删密能力**。
- 非 XML 的 FDF 格式。
- 完整富文本/XHTML 排版与字体样式回放。
- Reply/State 审阅关系与状态、BlendMode 的完整 PDF 映射及外观实现。
- 测量/云线/高亮型墨迹等 Intent 专用行为、云状边框，以及 bevelled/inset/underline 的显式 AP。

后续工作与 API 迁移见 [XFDF_UPGRADE_PROGRESS.md](XFDF_UPGRADE_PROGRESS.md)。
