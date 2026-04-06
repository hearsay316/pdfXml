# INSTALL_AND_PUBLISH_STATUS.md

这份文档面向两类读者：

1. **普通开发者**：想直接安装 CLI，或者在自己的 Rust 项目里调用库
2. **维护者**：想了解当前发布状态、检查方式，以及后续发布顺序

---

## 1. 当前推荐的使用方式

当前项目**优先推荐通过 Git 仓库使用**，包括：

- 直接安装 CLI
- 在 Rust 项目里通过 Git 依赖库 `pdfxml`

目前还**没有正式发布到 crates.io**。

---

## 2. 安装 CLI

### 2.1 通过 Git 直接安装

这是当前最推荐的 CLI 安装方式：

```bash
cargo install --git https://github.com/hearsay316/pdfXml.git --package pdfxml-cli
```

安装完成后，就可以直接使用：

```bash
pdfxml -i input.xfdf -o output.pdf
pdfxml --from-pdf -i annotated.pdf -o exported.xfdf
```

### 2.2 从源码构建 CLI

如果你想自己拉仓库、直接编译：

```bash
git clone https://github.com/hearsay316/pdfXml.git
cd pdfXml
cargo build --release -p pdfxml-cli
```

如果你是直接在仓库里运行 CLI：

```bash
cargo run -p pdfxml-cli -- --from-pdf -i examples/out.pdf -o exported.xfdf
cargo run -p pdfxml-cli -- -i exported.xfdf -t examples/WTchbOFP.pdf -o examples/out2.pdf -v
```

---

## 3. 在 Rust 项目里使用库

### 3.1 通过 Git 依赖接入

在你的 `Cargo.toml` 里加入：

```toml
[dependencies]
pdfxml = { git = "https://github.com/hearsay316/pdfXml.git" }
```

然后就可以在代码里直接使用：

```rust
use pdfxml::{load_xfdf, export_annotations};
```

### 3.2 一个最小例子

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, Some("original.pdf"), "annotated.pdf")?;
    Ok(())
}
```

### 3.3 从 PDF 读取注释并导出 XFDF

```rust
use pdfxml::{export_pdf_annotations_to_xfdf, load_annotations_from_pdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_annotations_from_pdf("annotated.pdf")?;
    println!("注释数量: {}", doc.annotations.len());

    export_pdf_annotations_to_xfdf("annotated.pdf", "annotated.xfdf")?;
    Ok(())
}
```

---

## 4. 仓库结构

当前项目已经拆成：

- 根库 crate：`pdfxml`
- CLI crate：`pdfxml-cli`
- 最终命令行二进制名：`pdfxml`

目录大致如下：

```text
pdfxml/
├── Cargo.toml              # 根库 crate + workspace 配置
├── cli/
│   ├── Cargo.toml          # CLI crate 配置
│   └── src/
│       └── main.rs         # CLI 薄壳入口
├── src/
│   ├── lib.rs              # SDK 入口
│   ├── xfdf.rs             # XFDF/XML 解析
│   ├── pdf.rs              # PDF 读写
│   ├── annotation.rs       # 注释模型
│   └── error.rs            # 错误类型
├── examples/               # 完整示例链路（保留）
└── tests/
    └── integration_test.rs
```

---

## 5. 为什么 examples 要完整保留

这个项目不仅是一个库，也是一条完整的 round-trip 示例链路：

- PDF -> XFDF/XML
- XFDF/XML -> PDF
- 真实示例输入/输出

因此当前发布策略是：

- **保留完整 examples**
- **不为了缩包主动删除关键 PDF/XFDF/XML 样例**

这意味着：

- 包会更大
- 但普通开发者拿到包后，能更完整地理解项目怎么用、怎么验证

---

## 6. 当前发布状态

### 6.1 当前状态

当前项目：

- 已经可以通过 Git 安装 CLI
- 已经可以通过 Git 依赖方式接入库
- 尚未正式发布到 crates.io

### 6.2 为什么现在不急着发 crates.io

原因不是结构不对，而是当前优先级更偏向：

- 保证项目完整可用
- 保留完整示例链路
- 先让开发者通过 Git 直接使用

### 6.3 如果以后发布到 crates.io

推荐发布顺序：

1. 先发布库包：`pdfxml`
2. 再发布 CLI 包：`pdfxml-cli`

原因：

- `pdfxml-cli` 依赖 `pdfxml = "0.1.0"`
- 所以要先让库包进入 registry，CLI 包才能正常完成独立发布校验

---

## 7. 当前检查方式

### 7.1 基础检查

在仓库里可直接运行：

```bash
cargo check -p pdfxml-cli
cargo test --workspace
cargo package -p pdfxml --allow-dirty --list
```

### 7.2 Windows / MSVC 环境

仓库里保留了现有辅助脚本：

- `run_vs_all_tests.cmd`
- `run_vs_package.cmd`
- `run_vs_tests.cmd`

它们主要用于：

- 在 Visual Studio 开发环境里跑测试
- 做库包打包验证

### 7.3 已确认的现状

当前已确认：

- `pdfxml` 库包可以完成 package 验证
- `pdfxml-cli` 在 `pdfxml` 尚未发布到 registry 之前，单独 `cargo package` 失败是预期现象

这不影响：

- Git 安装 CLI
- Git 依赖库
- 当前本地开发与测试

---

## 8. 推荐阅读顺序

如果你是第一次接触这个项目，建议这样看：

1. `README.md`：快速上手
2. `SDK_GUIDE.md`：理解库 API 与项目结构
3. `ANNOTATION_SUPPORT.md`：看当前支持哪些批注
4. `examples/`：看完整 round-trip 示例链路

---

## 9. 一句话总结

当前最稳妥的使用方式是：

- **CLI：通过 Git 安装 `pdfxml-cli`**
- **库：通过 Git 依赖 `pdfxml`**
- **发布：暂不急着 crates.io，等需要时按 `pdfxml` -> `pdfxml-cli` 顺序发布**
