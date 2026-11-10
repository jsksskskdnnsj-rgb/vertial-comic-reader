# Third-party notices (MH 漫画阅读器)

MH Reader 本体采用 [MIT License](./LICENSE)。以下列出与本 PR 引入的 **Rust 归档依赖**相关的许可证要点；完整文本请以各上游仓库为准。

## 核实来源（截至 2026-11）

| 组件 | 版本（`Cargo.lock`） | crates.io / 上游许可证 | MH 中的用途 |
| --- | --- | --- | --- |
| [sevenz-rust](https://github.com/dyz1990/sevenz-rust) | 0.6.1 | **Apache-2.0** | 仅在 `src-tauri/src/archives.rs` 调用 `decompress_file`，**导入时解压** `.7z` / `.cb7` |
| [unrar](https://github.com/muja/unrar.rs) | 0.5.8 | **MIT OR Apache-2.0**（贡献者部分） | 仅在 `archives.rs` 调用 `extract_to`，**导入时解压** `.rar` / `.cbr` |
| [unrar_sys](https://github.com/muja/unrar.rs) | 0.5.8 | **MIT OR Apache-2.0**（FFI 绑定）；内嵌 **UnRAR C++ 源码**另受下文 **unRAR 许可**约束 | 编译并链接 rarlab 官方 UnRAR 源码，供 `unrar` crate 解压 |

> **MH 声明：** 上述归档库仅用于**只读解压**漫画导入，不创建 RAR/7z 压缩包，不重新实现 RAR 专有压缩算法，不对外提供「制作 RAR 兼容压缩器」类功能。

---

## unRAR 许可（内嵌于 `unrar_sys` 的 C++ 源码）

`unrar` / `unrar_sys` 通过 Cargo 构建脚本编译 Alexander Roshal / rarlab 的 **UnRAR** 自由分发版源码。该源码**不是** MIT/Apache；适用下列 **unRAR freeware license**（与 [muja/unrar.rs 仓库内 `unrar_sys/vendor/unrar/license.txt`](https://github.com/muja/unrar.rs/blob/master/unrar_sys/vendor/unrar/license.txt) 一致）。

官方说明页：<https://www.rarlab.com/rar_add.htm>（「UNRAR source / license」条目）。

### 许可原文（英文）

```
 ******    *****   ******   UnRAR - free utility for RAR archives
 **   **  **   **  **   **  ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 ******   *******  ******    License for use and distribution of
 **   **  **   **  **   **   ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
 **   **  **   **  **   **         FREE portable version
                                   ~~~~~~~~~~~~~~~~~~~~~

      The source code of UnRAR utility is freeware. This means:

   1. All copyrights to RAR and the utility UnRAR are exclusively
      owned by the author - Alexander Roshal.

   2. UnRAR source code may be used in any software to handle
      RAR archives without limitations free of charge, but cannot be
      used to develop RAR (WinRAR) compatible archiver and to
      re-create RAR compression algorithm, which is proprietary.
      Distribution of modified UnRAR source code in separate form
      or as a part of other software is permitted, provided that
      full text of this paragraph, starting from "UnRAR source code"
      words, is included in license, or in documentation if license
      is not available, and in source code comments of resulting package.

   3. The UnRAR utility may be freely distributed. It is allowed
      to distribute UnRAR inside of other software packages.

   4. THE RAR ARCHIVER AND THE UnRAR UTILITY ARE DISTRIBUTED "AS IS".
      NO WARRANTY OF ANY KIND IS EXPRESSED OR IMPLIED.  YOU USE AT
      YOUR OWN RISK. THE AUTHOR WILL NOT BE LIABLE FOR DATA LOSS,
      DAMAGES, LOSS OF PROFITS OR ANY OTHER KIND OF LOSS WHILE USING
      OR MISUSING THIS SOFTWARE.

   5. Installing and using the UnRAR utility signifies acceptance of
      these terms and conditions of the license.

   6. If you don't agree with terms of the license you must remove
      UnRAR files from your storage devices and cease to use the
      utility.

      Thank you for your interest in RAR and UnRAR.

                                            Alexander L. Roshal
```

### 与 MH 相关的关键条款（中文摘要）

- **允许**：在软件中**处理（含解压）RAR 归档**，免费、无额外限制（条款 2 前半）。
- **禁止**：用于开发 **RAR（WinRAR）兼容的压缩/归档器**，或 **重新实现专有的 RAR 压缩算法**（条款 2 后半）。
- **MH 合规方式**：`archives::extract_rar` 仅在用户导入时将 `.rar` / `.cbr` **解压到本地数据目录**后以文件夹形式入库；应用不提供 RAR 打包、不调用任何 RAR 创建 API。

---

## sevenz-rust（Apache-2.0）

- 许可证全文：<http://www.apache.org/licenses/LICENSE-2.0>
- 上游 crate 同时具备压缩与解压 API；**MH 仅调用解压路径**（`sevenz_rust::decompress_file`），用于漫画 `.7z` / `.cb7` 导入，不在产品内提供 7z 创建功能。

---

## unrar / unrar_sys（Rust 层 MIT 或 Apache-2.0）

- 许可证文件：<https://github.com/muja/unrar.rs/blob/master/LICENSE-MIT>、<https://github.com/muja/unrar.rs/blob/master/LICENSE-APACHE>
- 上游文档明确：库**只能 extract / list**，不能 create archives（与 MH 用法一致）。

---

## 其他常见 Rust 依赖

书架/阅读核心长期使用的 `zip`（ZIP/CBZ 流式读取）等 crate 的许可证见各 crate 的 `Cargo.toml` 与 [crates.io](https://crates.io/) 页面；本文件不重复枚举全部传递依赖。发行二进制时建议运行 `cargo license`（或 CI 许可证扫描）生成完整 SPDX 清单。
