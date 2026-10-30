# MH漫画阅读器（Tauri 桌面版）v0.7.9

> **普通用户直接下载**：去 [Releases](https://github.com/jsksskskdnnsj-rgb/vertial-comic-reader/releases) 下载 `mh-reader.exe`，双击即用，无需编译。
> 开发者从源码构建请看下面的「构建运行」。

Windows 本地竖滑漫画阅读器（Tauri 2 + Rust + 原生 HTML/JS）。无边框窗口，默认约 1200×800，可托盘常驻。界面是柔光玻璃主题，中文 / English 可切换。

**已测试平台是 Windows 10/11**（系统自带 WebView2）。macOS / Linux 理论上能编，但没有测过。

[English README](README_EN.md)

## 功能

- **页面**：首页、书库、导入、书籍详情、设置、阅读页。入口是 `src/home.html`（旧的单页 `index.html` 已不存在）
- **三种导入**：ZIP / CBZ、文件夹（含子文件夹）、散图；也可以把文件拖进窗口
- **扫描文件夹**：选一个目录，每个含图片的直接子文件夹成为一本漫画，更里面的文件夹变成章节。先看结果再加入书架，已在书架里的路径不会重复添加。扫描在后台进行，可以取消，也可以一边读一边换页；每一页左下角都有进度，点一下回到扫描页。扫完会在应用里提示，并在系统允许时发一条通知，写明新增、合并和待确认的数量。同一文件夹没扫完，下次接着扫，扫完的会重新开始。打不开的图片记在页面上的问题列表里。压缩包和整盘扫描还不在这里，zip 仍走导入页
- **相似文件夹**：名字去掉卷号、话数、括号和标点后相同，而且每一本都有不重叠的顺序（第1–50话 / 第51–100话、卷1 / 卷2、上 / 下、part1 / part2）时，会自动合成一本，各文件夹按这个顺序变成章节。设置里可以关掉自动合并（默认开）。不太确定的会进待确认列表，能改书名、选分类、拖动或用按钮排序，也可以保持分开。书库里可以多选文件夹再合并，合并过的书可以拆回去。只改书架记录，不移动、不删除文件
- **按需阅读**：打开书只扫目录，翻到哪一页再解哪一页（extractor 队列 + 虚拟列表），整包不进内存
- **SQLite 书库**：只存路径不存副本；封面是能解码的靠前一页，缩到 240px
- **断点续读**、书签（锚在具体图片上）、分话（不分话 / 按文件夹 / 每 N 页）
- **阅读**：竖滑与翻页、自动滚动、页目录、宽度与间距。按 `?` 或工具栏上的说明按钮查看快捷键
- **分类**：书库左侧新建、改名、删除、调整顺序，并按分类筛选。导入和扫描可以指定分类，书籍页也能改。未分类和全部仍然在
- **书库**：封面显示阅读进度，卡片写着上次阅读时间。书很多时按页显示（每页 48 本）。搜索随输入过滤，`/` 或 Ctrl+K 聚焦搜索框
- **外观**：柔光主题、强调色、窗口背景与首页横幅分开设置、面板不透明度、壁纸适应方式、边框颜色
- **窗口**：自定义标题栏（可拖动、双击最大化）、关闭可藏到托盘。已经打开时再启动一次，会回到这个窗口
- 图片只在本机读取，不上传

### 能打开的图片

jpg、jpeg、jfif、jpe、png、webp、gif、bmp，以及 **avif（原样显示）**。

avif 页面可以由 WebView 直接画出来。本程序的 `image` 库没有打开 AVIF 解码，所以 avif 书可能没有封面，超长图切片和缩宽也会失败。svg、tif、heic、jxl 会在导入时拒绝。RAR / 7z 不支持，请先解压成文件夹。

## 构建运行

**Windows**（已测试）：

1. 安装 Rust（https://rustup.rs 的 `rustup-init.exe`）、Node.js LTS、VS 生成工具（C++ 桌面开发）。Win10/11 自带 WebView2。
2. 把本仓库克隆到任意目录，在根目录执行：
   ```
   python scripts/gen-icons.py   # 重绘全部图标（默认卡通变体 C；--variant a 或 b 换另外两版），会覆盖旧文件
   npm install                   # 只有第一次需要
   npm run build
   ```
3. 构建完成后，二进制会复制到目录第一层（`mh-reader.exe`），双击即可。代码没改就不必重新构建。

`scripts/apply-*.mjs` 是 v0.6–v0.7 把界面补丁写进源码用的一次性脚本，**不是**构建步骤。再跑它们可能盖掉柔光主题，需要时再跑 `scripts/apply-v07.mjs` 把柔光块放回样式表末尾。

## 目录结构

```
mh-reader/
  mh-reader.exe        # 构建后复制到这里
  mh-reader.db         # 书库（默认在程序旁边，可在设置里改）
  cache/               # 阅读页缓存
  covers/              # 240px 封面
  data-dir.txt         # 只有改过数据目录才有
  src/
    home.html          # 入口
    library.html       # 书库
    import.html        # 导入
    scan.html          # 扫描文件夹
    book.html          # 书籍详情
    settings.html      # 设置
    reader.html        # 阅读页（reader.js 虚拟列表）
    mh-ui.css          # 界面样式（末尾是 v0.7 柔光主题）
  archive/legacy-ui/   # v0.3 旧界面存档，不打进安装包
  src-tauri/
    tauri.conf.json
    src/               # 命令、extractor、SQLite、mhpage / mhcover / mhfile
  scripts/
    copy-exe.cjs       # 构建后把二进制拷到根目录
    gen-icons.py       # 重绘应用图标。默认变体 C（三页小塔）；--variant a|b 换另外两版
```

## 说明

- 首次构建会下载 Rust 依赖，需要几分钟。
- 无边框窗口：标题栏 — ▢ ✕ 为自定义按钮，空白处可拖拽移动窗口。关闭窗口默认藏到托盘，设置里可以改成直接退出。
- 自定义壁纸和横幅复制到数据目录，经 `mhfile` 协议读取（只允许 `hero-custom.*` 与 `banner-custom.*`）。漫画页走 `mhpage`，封面走 `mhcover`。

## 许可证

MIT，见 [LICENSE](LICENSE)。更新记录见 [CHANGELOG.md](CHANGELOG.md)。
