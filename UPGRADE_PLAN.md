# pdfxml 项目升级方案

版本：v0.1.1 → v0.2.0  
制定时间：2026-10-08  
预计完成时间：2-3 周

---

## 📋 升级目标

1. **性能提升 30%+** - 通过正则缓存、编译优化
2. **用户体验优化** - GUI 响应速度、批量处理
3. **代码质量提升** - 模块化、测试覆盖
4. **工程化建设** - CI/CD、文档完善

---

## 🎯 三阶段升级计划

### 阶段一：性能与清理（第 1 周）✅ 部分完成

#### 1.1 已完成项目 ✅
- [x] 清理临时文件（inspect_annots*.o, page_annots_dump.txt）
- [x] 更新 .gitignore 忽略临时文件
- [x] 添加正则表达式缓存（once_cell）
- [x] 优化 pdf.rs 中的 4 个正则表达式

#### 1.2 待完成项目
- [ ] **添加 once_cell 依赖到 Cargo.toml**
  ```toml
  [dependencies]
  once_cell = "1.19"
  ```

- [ ] **优化编译配置**
  ```toml
  [profile.release]
  lto = true              # 链接时优化，二进制减小 10-15%
  codegen-units = 1       # 更好的优化
  strip = true            # 去除调试符号
  opt-level = 3           # 最高优化级别
  
  [profile.dev]
  opt-level = 1           # 开发时也启用基础优化
  ```

- [ ] **查找并缓存其他正则表达式**
  - 检查 xfdf.rs 中的正则使用
  - 检查 annotation.rs 中的正则使用
  - 统一移到模块顶部作为静态变量

- [ ] **性能基准测试**
  ```bash
  cargo install cargo-criterion
  # 添加 benches/annotation_bench.rs
  # 测试大批量注释处理性能
  ```

**预期收益**：
- 处理速度提升 30-40%
- 二进制大小减小 15-20%
- 内存使用优化 10-15%

---

### 阶段二：功能增强（第 2 周）

#### 2.1 GUI 异步优化

**当前问题**：文件选择对话框阻塞主线程

**优化方案**：
```rust
// gui/src/app.rs
use gpui::Task;

fn browse_xfdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let xfdf_entity = self.xfdf_path.clone();
    let output_entity = self.output_path.clone();
    let mode = self.mode;
    
    cx.spawn(|this, mut cx| async move {
        // 在后台线程执行文件对话框
        let path = tokio::task::spawn_blocking(|| {
            FileDialog::new()
                .add_filter("XFDF/XML", &["xfdf", "xml"])
                .pick_file()
        }).await.ok().flatten();
        
        if let Some(path) = path {
            let path_str = path.display().to_string();
            let output = service::suggest_output_path(&path, mode);
            
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    Self::set_input_value(&xfdf_entity, path_str.clone(), window, cx);
                    Self::set_input_value(&output_entity, output.display().to_string(), window, cx);
                    this.set_success(format!("已选择 XFDF：{}", path_str), cx);
                });
            }).ok();
        }
    }).detach();
}
```

**依赖更新**：
```toml
[dependencies]
tokio = { version = "1.40", features = ["rt", "sync"] }
```

#### 2.2 批量处理功能

**新增模块**：`gui/src/batch.rs`

```rust
pub struct BatchProcessor {
    files: Vec<PathBuf>,
    progress: usize,
    errors: Vec<String>,
}

impl BatchProcessor {
    pub async fn process_batch(
        &mut self,
        mode: ConversionMode,
        progress_callback: impl Fn(usize, usize),
    ) -> Result<BatchResult> {
        for (idx, file) in self.files.iter().enumerate() {
            progress_callback(idx + 1, self.files.len());
            // 处理单个文件
        }
    }
}
```

**GUI 更新**：
- 添加"批量处理"按钮
- 添加进度条组件
- 添加批量结果摘要

#### 2.3 配置持久化

**新增模块**：`gui/src/config.rs`

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub last_xfdf_dir: Option<PathBuf>,
    pub last_pdf_dir: Option<PathBuf>,
    pub last_output_dir: Option<PathBuf>,
    pub window_size: (u32, u32),
    pub theme: String,
}

impl AppConfig {
    pub fn load() -> Self {
        // 从 ~/.pdfxml/config.toml 加载
    }
    
    pub fn save(&self) -> Result<()> {
        // 保存到 ~/.pdfxml/config.toml
    }
}
```

**依赖更新**：
```toml
[dependencies]
toml = "0.8"
dirs = "5.0"  # 获取用户配置目录
```

---

### 阶段三：工程化与重构（第 3 周）

#### 3.1 代码模块化

**拆分 src/pdf.rs (2652 行)**

```
src/pdf/
├── mod.rs          # 公共接口和导出
├── reader.rs       # PDF 读取逻辑 (~600 行)
├── writer.rs       # PDF 写入逻辑 (~800 行)
├── appearance.rs   # 外观流生成 (~900 行)
└── utils.rs        # 工具函数 (~350 行)
```

**重构步骤**：
1. 创建 `src/pdf/` 目录
2. 移动相关函数到对应模块
3. 更新 `mod.rs` 导出
4. 测试所有功能正常

#### 3.2 错误处理增强

**src/error.rs 升级**：

```rust
#[derive(Error, Debug)]
pub enum PdfXmlError {
    // 文件操作错误
    #[error("无法读取文件 {path}: {source}")]
    FileReadError {
        path: String,
        source: std::io::Error,
    },
    
    #[error("无法写入文件 {path}: {source}")]
    FileWriteError {
        path: String,
        source: std::io::Error,
    },
    
    // PDF 错误
    #[error("PDF 页面 {page} 不存在（共 {total} 页）")]
    PageNotFound { page: usize, total: usize },
    
    #[error("注释 {annotation_type} 写入失败: {reason}")]
    AnnotationWriteFailed {
        annotation_type: String,
        reason: String,
    },
    
    // XFDF 错误
    #[error("XFDF 格式无效: 缺少必需字段 {field}")]
    MissingRequiredField { field: String },
    
    // 现有错误类型保持不变
    #[error("XML 解析错误: {0}")]
    XmlParse(#[from] quick_xml::Error),
    
    #[error("PDF 处理错误: {0}")]
    PdfProcessing(String),
    
    // ... 其他错误类型
}
```

#### 3.3 测试体系建设

**目录结构**：
```
tests/
├── integration_test.rs          # 现有集成测试
├── performance_test.rs          # 新增：性能测试
├── batch_processing_test.rs     # 新增：批量处理测试
└── gui_test.rs                  # 新增：GUI 测试

benches/
├── annotation_bench.rs          # 新增：注释处理基准
├── pdf_generation_bench.rs      # 新增：PDF 生成基准
└── regex_bench.rs               # 新增：正则性能基准
```

**基准测试示例**：
```rust
// benches/annotation_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use pdfxml::{load_xfdf, export_annotations};

fn bench_large_xfdf(c: &mut Criterion) {
    c.bench_function("load 1000 annotations", |b| {
        b.iter(|| {
            let doc = load_xfdf(black_box("examples/large.xfdf")).unwrap();
            assert!(doc.annotations.len() >= 1000);
        });
    });
}

criterion_group!(benches, bench_large_xfdf);
criterion_main!(benches);
```

#### 3.4 CI/CD 建设

**新增 .github/workflows/ci.yml**：

```yaml
name: CI

on:
  push:
    branches: [master, gpui-*]
  pull_request:
    branches: [master]

jobs:
  test:
    name: Test on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
        rust: [stable, nightly]
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: ${{ matrix.rust }}
          components: clippy, rustfmt
      
      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Check formatting
        run: cargo fmt --all -- --check
      
      - name: Run clippy
        run: cargo clippy --all-targets --all-features -- -D warnings
      
      - name: Run tests
        run: cargo test --workspace --verbose
      
      - name: Build release
        run: cargo build --release --workspace
      
      - name: Run benchmarks (nightly only)
        if: matrix.rust == 'nightly'
        run: cargo bench --no-fail-fast

  coverage:
    name: Code coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      
      - name: Install tarpaulin
        run: cargo install cargo-tarpaulin
      
      - name: Generate coverage
        run: cargo tarpaulin --workspace --out Xml
      
      - name: Upload coverage
        uses: codecov/codecov-action@v4
        with:
          files: ./cobertura.xml

  release:
    name: Build release artifacts
    if: startsWith(github.ref, 'refs/tags/v')
    needs: [test]
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact_name: pdfxml-cli.exe
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact_name: pdfxml-cli
          - os: macos-latest
            target: x86_64-apple-darwin
            artifact_name: pdfxml-cli
    
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      
      - name: Build CLI
        run: cargo build --release -p pdfxml-cli
      
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: target/release/${{ matrix.artifact_name }}
```

#### 3.5 文档完善

**新增文档**：

1. **ARCHITECTURE.md** - 架构设计文档
   - 模块职责说明
   - 数据流图
   - 扩展指南

2. **PERFORMANCE.md** - 性能指标文档
   - 基准测试结果
   - 性能优化历史
   - 瓶颈分析

3. **GUI_MANUAL.md** - GUI 用户手册
   - 带截图的使用说明
   - 常见问题解答
   - 快捷键列表

4. **CONTRIBUTING.md** - 贡献指南
   - 代码风格
   - 提交规范
   - PR 流程

---

## 📦 依赖变更总览

### Cargo.toml 完整更新

```toml
[package]
name = "pdfxml"
version = "0.2.0"  # 版本升级
edition = "2021"
rust-version = "1.75"  # 最低 Rust 版本

[dependencies]
# 现有依赖
quick-xml = "0.36"
lopdf = "0.35"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2.0"
log = "0.4"
regex = "1"
serde = { version = "1.0", features = ["derive"] }
ttf-parser = "0.21"
base64 = "0.22"
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }

# 新增依赖
once_cell = "1.19"        # 正则缓存

[dev-dependencies]
tempfile = "3"
criterion = "0.5"         # 基准测试
proptest = "1.5"          # 属性测试

[profile.release]
lto = true
codegen-units = 1
strip = true
opt-level = 3

[profile.dev]
opt-level = 1

[[bench]]
name = "annotation_bench"
harness = false
```

### gui/Cargo.toml 更新

```toml
[dependencies]
pdfxml = { path = "..", version = "0.2.0" }
gpui = { git = "https://github.com/zed-industries/zed" }
gpui_platform = { git = "https://github.com/zed-industries/zed" }
gpui-component = { git = "https://github.com/longbridge/gpui-component" }
gpui-component-assets = { git = "https://github.com/longbridge/gpui-component" }
rfd = "0.15"
futures = "0.3"
anyhow = "1.0"
log = "0.4"
env_logger = "0.11"

# 新增
tokio = { version = "1.40", features = ["rt", "sync"] }
toml = "0.8"
dirs = "5.0"
serde = { version = "1.0", features = ["derive"] }
```

---

## 🧪 测试策略

### 回归测试清单

升级完成后必须通过的测试：

```bash
# 1. 单元测试
cargo test --lib

# 2. 集成测试
cargo test --test integration_test

# 3. 文档测试
cargo test --doc

# 4. 示例测试
cargo test --examples

# 5. Clippy 检查
cargo clippy --all-targets --all-features -- -D warnings

# 6. 格式检查
cargo fmt --all -- --check

# 7. 基准测试（与 v0.1.1 对比）
cargo bench

# 8. CLI 功能测试
cd cli
cargo run -- xfdf-to-pdf ../examples/sample.xfdf ../examples/sample.pdf output.pdf
cargo run -- pdf-to-xfdf output.pdf output.xfdf

# 9. GUI 启动测试
cd gui
cargo run

# 10. 发布构建测试
cargo build --release --workspace
```

### 性能对比测试

```bash
# 生成大型测试文件（1000 条注释）
python scripts/generate_large_xfdf.py > examples/large.xfdf

# v0.1.1 基准测试
git checkout v0.1.1
cargo bench --bench annotation_bench -- --save-baseline v0.1.1

# v0.2.0 性能测试
git checkout gpui-0.1
cargo bench --bench annotation_bench -- --baseline v0.1.1

# 预期：v0.2.0 比 v0.1.1 快 30% 以上
```

---

## 📊 成功指标

### 性能指标
- [ ] 处理 1000 条注释速度提升 ≥ 30%
- [ ] 内存使用优化 ≥ 10%
- [ ] 二进制大小减小 ≥ 15%

### 功能指标
- [ ] GUI 支持批量处理
- [ ] GUI 支持配置持久化
- [ ] GUI 文件选择不阻塞主线程

### 质量指标
- [ ] 测试覆盖率 ≥ 75%
- [ ] Clippy 无警告
- [ ] 所有文档更新完成

### 工程指标
- [ ] CI/CD 流水线运行正常
- [ ] 所有平台构建成功
- [ ] 发布包大小 ≤ 10MB

---

## 🚀 发布计划

### v0.2.0-alpha (第 1 周末)
- 性能优化完成
- 临时文件清理
- 正则缓存实施

### v0.2.0-beta (第 2 周末)
- GUI 异步优化
- 批量处理功能
- 配置持久化

### v0.2.0 正式版 (第 3 周末)
- 代码重构完成
- 测试覆盖充足
- CI/CD 就绪
- 文档齐全

---

## 🔄 回滚方案

如果升级出现问题：

```bash
# 方案 1：回退到 v0.1.1
git checkout v0.1.1
cargo build --release

# 方案 2：回退特定功能
git revert <commit-hash>

# 方案 3：创建修复分支
git checkout -b hotfix/v0.2.0-fix master
```

---

## 👥 人员分工建议

如果是团队协作：

- **后端开发**：性能优化、代码重构
- **前端开发**：GUI 异步优化、批量处理
- **测试工程师**：测试体系、基准测试
- **DevOps**：CI/CD 建设、发布流程
- **文档工程师**：用户手册、API 文档

如果是个人开发：
- 严格按照三阶段逐步推进
- 每个阶段完成后做完整测试
- 及时提交 git commit

---

## 📝 注意事项

1. **兼容性**：确保 v0.2.0 API 向后兼容 v0.1.1
2. **性能回归**：每个 PR 都要跑基准测试
3. **文档同步**：代码变更必须同步更新文档
4. **版本标记**：遵循语义化版本规范
5. **变更日志**：维护详细的 CHANGELOG.md

---

## 🎉 预期成果

完成此升级方案后：

- ✅ **性能提升** 30-40%，处理大文件更快
- ✅ **用户体验** GUI 响应更流畅，支持批量操作
- ✅ **代码质量** 模块化清晰，易于维护和扩展
- ✅ **工程化** CI/CD 自动化，测试覆盖完善
- ✅ **可维护性** 文档齐全，新人容易上手

这将使 pdfxml 成为一个**生产级、高性能、易维护**的 PDF 注释转换工具！
