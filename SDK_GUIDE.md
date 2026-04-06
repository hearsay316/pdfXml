# pdfxml SDK 详细说明

这份文档是给“第一次接触这个项目的人”准备的。

你可以把 `pdfxml` 理解成两种用法：

1. **命令行工具**：直接在终端里运行
2. **Rust SDK / library**：在你自己的 Rust 项目里当库来调用

---

## 1. 这个项目到底在做什么

很多 PDF 标注工具可以把注释单独导出成 `XFDF` 或 `XML` 文件。

比如：
- 高亮
- 便签
- 删除线
- 线条
- 矩形
- 圆形
- 多边形

这些信息本身不是 PDF 页面内容，而是“注释数据”。

`pdfxml` 的工作就是：

- **读取 XFDF/XML 注释文件**
- **把它解析成 Rust 里的数据结构**
- **再把这些注释写回 PDF**

所以它像一个“翻译器”：

- 输入：XFDF/XML
- 输出：PDF

---

## 2. CLI 和 SDK 的区别

### CLI 用法

适合你只想直接转换文件：

```bash
pdfxml -i annotations.xfdf -o output.pdf
```

### SDK 用法

适合你想在自己的 Rust 程序里调用：

```rust
use pdfxml::{load_xfdf, export_annotations};
```

比如你的系统里已经有：
- 文件上传流程
- 批量处理任务
- Web API
- 桌面软件

那你就不一定想再绕一层命令行，而是想在代码里直接调用库函数。

---

## 3. 项目结构怎么理解

现在项目大致可以这样理解：

```text
src/
├── lib.rs         # 对外暴露的 SDK 入口
├── main.rs        # 命令行入口（很薄，只负责接参数和调用库）
├── xfdf.rs        # 解析 XFDF/XML
├── pdf.rs         # 生成 PDF / 合并注释到 PDF
├── annotation.rs  # 注释的数据结构定义
└── error.rs       # 错误类型
```

### `src/lib.rs`

这是库的门面。

外部项目通常不需要知道内部每个文件怎么组织，直接从这里导入就行：

```rust
use pdfxml::{XfdfDocument, PdfAnnotationExporter};
```

### `src/main.rs`

这是命令行入口。

它自己不做复杂业务，只做：

1. 读取参数
2. 调用 `load_xfdf`
3. 调用 `export_annotations`
4. 打印结果

### `src/xfdf.rs`

负责把 XML 文本解析成 Rust 结构体。

你可以把它理解成“把字符串变成程序能看懂的数据”。

### `src/pdf.rs`

负责把注释真正写进 PDF。

这里是导出逻辑最核心的地方。

### `src/annotation.rs`

定义各种注释长什么样。

比如：
- Text
- Highlight
- Square
- Circle
- Line
- Polygon

### `src/error.rs`

统一定义错误类型。

这样不管是：
- 读文件失败
- XML 解析失败
- PDF 处理失败

最后都能用比较统一的方式返回。

---

## 4. 最常用的公开 API

目前最重要的入口主要有这几个。

## 4.1 `XfdfDocument::parse`

作用：

**把一段 XFDF/XML 字符串解析成 `XfdfDocument`。**

示例：

```rust
use pdfxml::XfdfDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf)?;
    println!("注释数量: {}", doc.annotations.len());
    Ok(())
}
```

适合场景：
- 你已经拿到了 XFDF 字符串
- 内容来自网络、数据库、接口返回值，而不是本地文件

---

## 4.2 `load_xfdf`

作用：

**直接从文件读取并解析 XFDF。**

示例：

```rust
use pdfxml::load_xfdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    println!("注释数量: {}", doc.annotations.len());
    Ok(())
}
```

它本质上等于：

1. `read_to_string`
2. `XfdfDocument::parse`

只是帮你少写两步。

适合场景：
- 你手里就是一个 `.xfdf` 文件
- 想快速导入

---

## 4.3 `PdfAnnotationExporter::new`

作用：

**创建一个导出器。**

你可以把它理解成“准备一个负责写 PDF 的工具对象”。

示例：

```rust
use pdfxml::PdfAnnotationExporter;

let mut exporter = PdfAnnotationExporter::new();
```

通常它会和下面两个函数配合使用：
- `export_to_new_pdf`
- `export_to_existing_pdf`

---

## 4.4 `export_to_new_pdf`

作用：

**新建一个 PDF，把注释写进去。**

示例：

```rust
use pdfxml::{PdfAnnotationExporter, XfdfDocument};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf)?;
    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, Path::new("output.pdf"))?;
    Ok(())
}
```

适合场景：
- 你只想把注释内容单独导出成 PDF
- 不需要基于已有 PDF 合并

---

## 4.5 `export_to_existing_pdf`

作用：

**把注释合并到一个已经存在的 PDF 里。**

示例：

```rust
use pdfxml::{PdfAnnotationExporter, XfdfDocument};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = std::fs::read_to_string("annotations.xfdf")?;
    let doc = XfdfDocument::parse(&xfdf)?;

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_existing_pdf(
        &doc,
        Path::new("original.pdf"),
        Path::new("annotated.pdf"),
    )?;

    Ok(())
}
```

适合场景：
- 你已经有原始 PDF
- 想把 XFDF 注释覆盖/合并回去

---

## 4.6 `export_annotations`

作用：

**一个更省事的顶层包装函数。**

示例：

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;

    export_annotations(
        &doc,
        Some("original.pdf"),
        "annotated.pdf",
    )?;

    Ok(())
}
```

它的规则是：

- `Some("original.pdf")` → 合并到已有 PDF
- `None::<&str>` → 新建 PDF

新建 PDF 的写法示例：

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, None::<&str>, "output.pdf")?;
    Ok(())
}
```

适合场景：
- 你不想自己手动 new 一个 exporter
- 想用最短代码完成导出

---

## 5. 两种推荐调用方式

## 方式 A：最容易上手

如果你只想“读取 xfdf 然后导出 pdf”，推荐这样写：

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, None::<&str>, "output.pdf")?;
    Ok(())
}
```

特点：
- 代码最短
- 最适合新手
- 足够覆盖很多常见场景

---

## 方式 B：更明确、更可控

如果你希望代码更清楚地表达“先解析，再导出”，推荐这样写：

```rust
use pdfxml::{PdfAnnotationExporter, XfdfDocument};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = std::fs::read_to_string("annotations.xfdf")?;
    let doc = XfdfDocument::parse(&xfdf)?;

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_existing_pdf(
        &doc,
        Path::new("original.pdf"),
        Path::new("annotated.pdf"),
    )?;

    Ok(())
}
```

特点：
- 过程更清楚
- 更适合以后继续扩展
- 调试时更容易看出哪一步出错

---

## 6. 一个完整思路：程序内部是怎么流动的

可以把整个过程想成下面这样：

```text
XFDF 文件 / XFDF 字符串
        ↓
XfdfDocument::parse / load_xfdf
        ↓
得到 XfdfDocument
        ↓
PdfAnnotationExporter / export_annotations
        ↓
生成新的 PDF 或合并到已有 PDF
```

如果你以后要接自己的业务系统，通常只要在这条链上接入即可。

例如：

- 上传接口收到 XFDF 文件
- 后端调用 `load_xfdf` 或 `parse`
- 然后调用导出函数
- 最后把生成的 PDF 返回给前端或存盘

---

## 7. 常见问题

### 7.1 我该用 `parse` 还是 `load_xfdf`？

看你的输入来源：

- 已经是字符串 → 用 `XfdfDocument::parse`
- 是文件路径 → 用 `load_xfdf`

简单记：
- `parse` 处理“文本”
- `load_xfdf` 处理“文件”

---

### 7.2 我该用 `export_annotations` 还是 `PdfAnnotationExporter`？

如果你是第一次接触这个库：
- **先用 `export_annotations`**

如果你想更明确地控制导出流程：
- **用 `PdfAnnotationExporter`**

简单记：
- `export_annotations` 更省事
- `PdfAnnotationExporter` 更底层一点

---

### 7.3 为什么还保留 `main.rs`？

因为这个项目不只是库，还要能当命令行工具直接运行。

所以现在是“双入口”结构：

- `lib.rs`：给 Rust 代码调用
- `main.rs`：给命令行调用

这样复用同一套核心逻辑，不需要写两份实现。

---

### 7.4 如果我要以后发布 SDK，这种结构有什么好处？

好处主要有这些：

1. **公开入口更清楚**
   - 外部用户直接从 `lib.rs` 导入
2. **CLI 不会和核心逻辑搅在一起**
   - 命令行只是外壳
3. **更容易写测试**
   - 集成测试可以直接测库 API
4. **以后更容易发 crates.io**
   - 因为已经具备标准的 library crate 结构

---

## 8. 给新接手项目的人一个建议

如果你是第一次看这个项目，推荐按这个顺序读：

1. 先看 `src/lib.rs`
   - 知道对外暴露了什么
2. 再看 `src/main.rs`
   - 知道命令行怎么调用库
3. 再看 `src/xfdf.rs`
   - 知道注释怎么解析
4. 最后看 `src/pdf.rs`
   - 知道注释怎么写进 PDF

这样会比一上来直接啃 `pdf.rs` 更容易懂。

---

## 9. 一个最短可运行 SDK 示例

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("examples/sample.xfdf")?;
    export_annotations(&doc, None::<&str>, "output.pdf")?;
    Ok(())
}
```

如果这段你已经看懂了，说明你已经掌握这个 SDK 最核心的用法了。

---

## 10. 后续扩展时怎么想

以后如果继续扩展 SDK，建议保持这个思路：

- **对外 API 尽量简单**
- **内部实现可以复杂，但不要把复杂度直接暴露给调用者**
- **CLI 继续做薄壳**
- **优先保证 `lib.rs` 的公开接口稳定**

这样后面无论是：
- 发 crate
- 做 Web 服务
- 做桌面工具
- 做批处理系统

都更容易复用。
