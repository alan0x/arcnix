use std::collections::BTreeMap;
use std::fmt::Write as _;

use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    Outline,
    Filled,
}

impl Style {
    pub const ALL: [Self; 2] = [Self::Outline, Self::Filled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Filled => "Filled",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Filled => "filled",
        }
    }

    pub fn from_id(id: &str) -> Self {
        split_id(id).0
    }
}

pub(super) struct Icon {
    pub id: &'static str,
    source: &'static str,
    pub label: &'static str,
    keywords: &'static str,
}

impl Icon {
    const fn new(
        id: &'static str,
        source: &'static str,
        label: &'static str,
        keywords: &'static str,
    ) -> Self {
        Self {
            id,
            source,
            label,
            keywords,
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let label = self.label.to_ascii_lowercase();
        query.split_whitespace().all(|term| {
            label.contains(term) || self.source.contains(term) || self.keywords.contains(term)
        })
    }

    pub fn identifier(&self, style: Style) -> String {
        match style {
            Style::Outline => self.id.to_owned(),
            Style::Filled => format!("filled:{}", self.id),
        }
    }

    pub fn class(&self, style: Style) -> String {
        format!("folder-icon-{}-{}", style.slug(), self.id)
    }
}

pub(super) struct Group {
    pub label: &'static str,
    pub icons: &'static [Icon],
}

impl Group {
    pub fn matches(&self, query: &str) -> bool {
        self.icons.iter().any(|icon| icon.matches(query))
    }
}

#[rustfmt::skip]
pub(super) const GROUPS: &[Group] = &[
    Group {
        label: "Files & work",
        icons: &[
            Icon::new("projects", "puzzle-piece", "Projects", "puzzle 项目 拼图"),
            Icon::new("work", "briefcase", "Work", "office business 工作 办公 公文包"),
            Icon::new("documents", "file-text", "Documents", "files text 文档 文件 文本"),
            Icon::new("notes", "note-pencil", "Notes", "writing pencil 笔记 备忘 写作"),
            Icon::new("books", "books", "Books", "library reading 图书 阅读 书籍"),
            Icon::new("calendar", "calendar", "Calendar", "schedule date 日历 日期 行程"),
            Icon::new("tasks", "list-checks", "Task list", "tasks todo checklist 任务 待办 清单"),
            Icon::new("favorites", "star", "Favorites", "bookmark rating 收藏 星标 星星"),
            Icon::new("shared", "users-three", "Shared", "people team collaboration 共享 分享 用户 团队 人员"),
            Icon::new("mail", "envelope", "Mail", "email inbox 邮件 信封 邮箱"),
        ],
    },
    Group {
        label: "Media & design",
        icons: &[
            Icon::new("photos", "camera", "Photos", "images pictures album 照片 摄影 相机 相册 图片"),
            Icon::new("images", "image", "Images", "picture landscape wallpaper 图片 图像 壁纸"),
            Icon::new("music", "music-notes", "Music", "audio songs 音乐 歌曲 音频 音符"),
            Icon::new("videos", "film-slate", "Videos", "movie cinema 视频 电影 影片"),
            Icon::new("headphones", "headphones", "Headphones", "audio podcast 耳机 播客 音频"),
            Icon::new("design", "palette", "Design", "art paint 创作 设计 绘画 调色板"),
            Icon::new("presentation", "presentation-chart", "Presentation", "slides chart 演示 幻灯片 图表"),
            Icon::new("games", "game-controller", "Games", "controller 游戏 手柄"),
        ],
    },
    Group {
        label: "Storage & transfers",
        icons: &[
            Icon::new("archive", "archive", "Archive", "zip compressed 归档 压缩 存档"),
            Icon::new("downloads", "download-simple", "Download", "downloads receive 下载 接收"),
            Icon::new("uploads", "upload-simple", "Upload", "uploads send 上传 发送"),
            Icon::new("cloud", "cloud", "Cloud", "sync cloud storage 云 云端 云盘 同步"),
            Icon::new("backups", "floppy-disk", "Backups", "restore save 备份 恢复 软盘"),
            Icon::new("data", "database", "Data", "sql database 数据 数据库"),
            Icon::new("drive", "hard-drive", "Drive", "disk storage 磁盘 硬盘 存储"),
            Icon::new("package", "package", "Package", "box bundle 包裹 软件包 纸箱"),
        ],
    },
    Group {
        label: "Development & systems",
        icons: &[
            Icon::new("code", "code", "Code", "source programming 代码 源码 编程 开发"),
            Icon::new("terminal", "terminal-window", "Terminal", "ssh shell command 终端 命令行"),
            Icon::new("server", "hard-drives", "Server", "host machine rack 主机 服务器"),
            Icon::new("network", "globe", "Network", "internet web globe 网络 互联网 地球"),
            Icon::new("tools", "wrench", "Tools", "repair utilities 工具 扳手 维修"),
            Icon::new("settings", "gear", "Settings", "configuration preferences 设置 配置 齿轮"),
            Icon::new("security", "lock-key", "Security", "lock private safe 安全 私密 锁"),
            Icon::new("key", "key", "Key", "credentials token 密钥 钥匙 凭据"),
            Icon::new("logs", "clipboard-text", "Logs", "history records reports 日志 记录 报告"),
            Icon::new("bug", "bug", "Bug", "debug issue defect 调试 缺陷 虫子"),
            Icon::new("link", "link-simple", "Link", "alias url 链接 快捷方式 替身"),
        ],
    },
    Group {
        label: "Personal",
        icons: &[
            Icon::new("home", "house", "Home", "house 家 主页 主目录"),
            Icon::new("heart", "heart", "Heart", "love 爱心 心形"),
            Icon::new("pin", "map-pin", "Location", "map address 位置 地点 地图 地址"),
            Icon::new("travel", "airplane", "Travel", "flight travel 旅行 出行 飞机"),
            Icon::new("education", "graduation-cap", "Education", "school study 学习 教育 学校"),
            Icon::new("plant", "plant", "Plant", "nature gardening 植物 自然 园艺"),
            Icon::new("pets", "paw-print", "Pets", "animals paw 宠物 动物 爪印"),
            Icon::new("shopping", "shopping-bag", "Shopping", "bag purchases 购物 采购 袋子"),
        ],
    },
];

fn split_id(id: &str) -> (Style, &str) {
    id.strip_prefix("filled:")
        .map_or((Style::Outline, id), |name| (Style::Filled, name))
}

pub(super) fn class(id: &str) -> Option<String> {
    let (style, name) = split_id(id);
    GROUPS
        .iter()
        .flat_map(|group| group.icons)
        .find(|icon| icon.id == name)
        .map(|icon| icon.class(style))
}

pub(super) fn css() -> String {
    let assets = include_str!("../assets/vendor/phosphor/folder-icons.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let (name, shapes) = line.split_once('\t').expect("Phosphor icon name");
            (
                name,
                shapes.split_once('\t').expect("Phosphor icon variants"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut css = String::new();
    for icon in GROUPS.iter().flat_map(|group| group.icons) {
        let (outline, filled) = assets[icon.source];
        for (style, svg) in Style::ALL.into_iter().zip([outline, filled]) {
            writeln!(
                css,
                ".{}{{--folder-icon-mask:url(\"data:image/svg+xml;base64,{}\")}}",
                icon.class(style),
                STANDARD.encode(svg),
            )
            .unwrap();
        }
    }
    css
}
