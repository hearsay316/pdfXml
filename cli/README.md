# pdfxml-cli

[English](#english) | [中文](#中文)

---

## 中文

`pdfxml-cli` 是 `pdfxml` 项目的命令行工具。

注意：
- 命令行工具包名是 **`pdfxml-cli`**
- 安装后可执行文件名是 **`pdfxml`**
- 库能力由 **`pdfxml`** crate 提供

### 安装

```bash
cargo install pdfxml-cli
```

### 功能

- XFDF/XML -> 新 PDF
- XFDF/XML -> 合并到现有 PDF
- PDF -> XFDF
- 支持常见注释类型导入/导出

### 用法

#### XFDF -> PDF

```bash
pdfxml -i annotations.xfdf -o output.pdf
```

#### XFDF -> 现有 PDF

```bash
pdfxml -i annotations.xfdf -t original.pdf -o annotated.pdf
```

#### PDF -> XFDF

```bash
pdfxml --from-pdf -i annotated.pdf -o exported.xfdf
```

#### 详细日志

```bash
pdfxml -i annotations.xfdf -t original.pdf -o annotated.pdf -v
```

### 参数

- `-i, --input <FILE>`：输入文件
- `-o, --output <FILE>`：输出文件
- `-t, --target-pdf <FILE>`：目标 PDF
- `--from-pdf`：从 PDF 导出 XFDF
- `-v, --verbose`：详细日志

### 路径解析

CLI 支持在 workspace 根目录或 `cli/` 子目录中运行。
对于相对路径，会优先按当前目录解析；如果找不到，也会尝试按上一级目录解析。

### 中文 FreeText 字体

如需显式指定 CJK 字体，可设置：

```bash
PDFXML_CJK_FONT=/path/to/font.ttf
```

### 许可证

本项目采用 MIT 许可证。详情见仓库中的 `LICENSE` 文件。

### 相关项目

- 库 crate：`pdfxml`
- API 文档：<https://docs.rs/pdfxml>
- CLI API 文档：<https://docs.rs/pdfxml-cli>
- 仓库：<https://github.com/hearsay316/pdfXml.git>

---

## English

`pdfxml-cli` is the command-line tool for the `pdfxml` project.

Important:
- The package name is **`pdfxml-cli`**
- The installed executable name is **`pdfxml`**
- Library APIs are provided by the **`pdfxml`** crate

### Installation

```bash
cargo install pdfxml-cli
```

### Features

- XFDF/XML -> new PDF
- XFDF/XML -> existing PDF
- PDF -> XFDF
- Import/export support for common annotation types

### Usage

#### XFDF -> PDF

```bash
pdfxml -i annotations.xfdf -o output.pdf
```

#### XFDF -> existing PDF

```bash
pdfxml -i annotations.xfdf -t original.pdf -o annotated.pdf
```

#### PDF -> XFDF

```bash
pdfxml --from-pdf -i annotated.pdf -o exported.xfdf
```

#### Verbose logging

```bash
pdfxml -i annotations.xfdf -t original.pdf -o annotated.pdf -v
```

### Arguments

- `-i, --input <FILE>`: input file
- `-o, --output <FILE>`: output file
- `-t, --target-pdf <FILE>`: target PDF
- `--from-pdf`: export XFDF from PDF
- `-v, --verbose`: verbose logging

### Path resolution

The CLI can be run from the workspace root or from the `cli/` subdirectory.
For relative paths, it first resolves against the current directory, then falls back to the parent directory when needed.

### CJK FreeText fonts

To explicitly specify a CJK font:

```bash
PDFXML_CJK_FONT=/path/to/font.ttf
```

### License

This project is licensed under the MIT License. See the `LICENSE` file in the repository for details.

### Related crates

- Library crate: `pdfxml`
- API docs: <https://docs.rs/pdfxml>
- CLI API docs: <https://docs.rs/pdfxml-cli>
- Repository: <https://github.com/hearsay316/pdfXml.git>
