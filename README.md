# MH漫画阅读器（Tauri 桌面版）v0.3.1

> **普通用户直接下载**：去 [Releases](https://github.com/jsksskskdnnsj-rgb/vertial-comic-reader/releases) 下载 `mh-reader.exe`，双击即用，无需编译。
> 开发者从源码构建请看下面的「构建运行」。

浅紫薰衣草主题的本地漫画阅读器：首页（欢迎横幅 + 拖放导入 + 最近阅读）、书架、本地导入、设置。

[English README](README_EN.md)

## 功能

- **三种导入**：ZIP / CBZ 压缩包、文件夹、散图；支持把文件直接拖进窗口导入
- **ZIP 流式读取**：只读中央目录，翻到哪页 Rust 按需解哪页到缓存，整包不进内存，大包也能开
- **SQLite 本地书架**：只存路径不存副本；封面=阅读顺序第一张
- **断点续读**：阅读进度每 4 秒自动保存
- **书架目录**：新建 / 重命名 / 删除目录，图书可移入移出
- **主题**：5 套预设 + 自定义取色 + 首页横幅可换
- **阅读设置**：默认竖滑/翻页、省内存模式开关
- **数据目录可自定义**：默认在 exe 旁边（便携），设置页可更改
- 图片只在本地读取，不联网、不上传

## 构建运行

**Windows**（已测试）：

1. 安装 Rust（https://rustup.rs 的 `rustup-init.exe`）、Node.js LTS、VS 生成工具（C++ 桌面开发）。Win10/11 自带 WebView2。
2. 把本仓库 **解压/克隆到任意目录**（文件直接在根目录，不要再套一层文件夹），进到该目录执行：
   ```
   python scripts/gen-icons.py   # 生成应用图标（只需一次）
   npm install        # 只有第一次需要
   npm run build
   ```
3. 构建完成后，二进制会自动复制到目录第一层（Windows 为 `mh-reader.exe`），双击它就能用。
   以后日常使用直接双击这个文件，只有代码更新了才需要重新 `npm run build`。

**macOS / Linux**：理论上 `npm run build` 即可（Tauri 跨平台），但尚未测试，欢迎反馈。

## 目录结构

```
mh-reader/
  mh-reader(.exe)      # 构建后自动复制到这里，双击运行
  mh-reader.db         # 书架数据库（默认在程序旁边）
  page_cache/          # ZIP 单页解压缓存
  data-dir.txt         # （可选）自定义数据目录时才有
  src/
    index.html         # 书架主页（首页/书架/导入/设置，横幅图已内联为 data URI）
    reader.html        # 阅读页
    reader-boot.js     # 阅读页启动：从书架加载 + 应用阅读配置
    mh-common.js       # 共享库（invoke 封装、模糊排序、封面生成）
  src-tauri/
    tauri.conf.json
    src/main.rs        # Rust 命令：文件选择 / ZIP 流式 / SQLite 书架 / 窗口控制
    icons/             # 由 scripts/gen-icons.py 生成（不进仓库）
  scripts/
    copy-exe.cjs       # 构建后把二进制拷到根目录
    gen-icons.py       # 生成应用图标
```

## 说明

- 首次构建会下载 Rust 依赖，需要几分钟，耐心等。
- RAR / 7z 暂不支持，请先解压为文件夹后导入。
- 无边框窗口：标题栏 — ▢ ✕ 为自定义按钮，空白处可拖拽移动窗口。

## 许可证

MIT，见 [LICENSE](LICENSE)。更新记录见 [CHANGELOG.md](CHANGELOG.md)。
