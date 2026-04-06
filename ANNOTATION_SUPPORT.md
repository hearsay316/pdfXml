# 批注支持清单

本文档记录当前项目对 PDF/XFDF 批注类型的实现状态，区分：

- 已实现：XFDF 解析、PDF 导出、PDF 回读已基本打通
- 部分实现：只支持部分字段，或只在某一方向支持
- 未实现：尚未真正建模/导出/回读

## 已实现的批注类型

以下类型当前已经在项目内有实际数据结构与处理逻辑，通常支持 XFDF -> PDF -> XFDF 的基本 round-trip：

- `text`
- `highlight`
- `underline`
- `strikeout`
- `squiggly`
- `freetext`
- `square`
- `circle`
- `line`
- `polygon`
- `polyline`
- `ink`
- `stamp`
- `popup`

### 说明

#### `freetext`
- 已支持 `defaultstyle`
- 已支持 `defaultappearance`
- 已支持 `TextColor`
- 已支持 `align`
- 目前不是完整富文本支持，只覆盖当前项目实现的文字外观子集

#### `stamp`
- 已支持 `icon`
- 已支持 `imagedata`
- 目前对不同来源 PDF 的 stamp 图片外观回提不保证 100% 完整兼容

#### 图形类批注
以下图形类批注已经实现基础字段与 PDF 外观生成：
- `square`
- `circle`
- `line`
- `polygon`
- `polyline`
- `ink`

## 部分实现 / 有限制的支持

### `contents-richtext`
- 当前会尝试提取纯文本
- 不等于完整支持 XFDF/Adobe rich text
- 复杂 XHTML/HTML 样式当前未完整实现

### 通用扩展属性
- 未识别的属性会保存在内部 `extra` 字段中
- 但这不表示这些属性已经被 PDF 导出/回读逻辑真正使用

## 未实现的批注类型

以下类型目前在项目里还没有真正实现为可用功能：

- `caret`
- `fileattachment`
- `sound`
- `link`
- `widget`

### 当前状态说明
这些标签在 XFDF 解析白名单里可能已经出现，但当前仍属于：
- 没有完整数据结构
- 或没有真正的 XFDF 构建逻辑
- 或没有 PDF 导出逻辑
- 或没有 PDF 回读逻辑

因此不能视为“已支持”。

## 其他未完成能力

除批注类型本身外，当前还有以下能力未完整支持：

- FDF（非 XML）格式
- 表单字段填充（fields）
- widget 相关表单能力
- 完整 rich text / XHTML 内容
- 更完整的高级注释属性（如 reply/state/callout/border/dash 等）

## 当前结论

如果目标是处理常见批注类型，当前项目已经覆盖大部分常用场景。

如果目标是宣称“完整支持 XFDF 批注规范”，当前还不够，主要缺口在：

- `link`
- `fileattachment`
- `sound`
- `caret`
- `widget` / 表单字段
- rich text
- 更完整的高级属性兼容
