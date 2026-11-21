use std::collections::BTreeMap;
use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub(crate) struct Icon {
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

    pub fn class(&self) -> String {
        format!("folder-icon-{}", self.id)
    }
}

pub(crate) struct Group {
    pub label: &'static str,
    pub icons: &'static [Icon],
    colors: [&'static str; 3],
}

impl Group {
    pub fn matches(&self, query: &str) -> bool {
        self.icons.iter().any(|icon| icon.matches(query))
    }
}

#[rustfmt::skip]
pub(crate) const GROUPS: &[Group] = &[
    Group {
        label: "Files & work",
        colors: ["#ffdd83", "#f0ad44", "#ae6529"],
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
        colors: ["#fcb7ef", "#c979dd", "#844da8"],
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
        colors: ["#87e7ff", "#429fd5", "#3165ba"],
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
        colors: ["#a5f4d8", "#47bcae", "#1c7d85"],
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
        colors: ["#ffc0a1", "#fb8a76", "#b64365"],
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

pub(crate) fn name(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, name)| name)
}

pub(crate) fn class(id: &str) -> Option<String> {
    GROUPS
        .iter()
        .flat_map(|group| group.icons)
        .find(|icon| icon.id == name(id))
        .map(Icon::class)
}

fn centered_mask(source: &str, svg: &str) -> String {
    // These Phosphor silhouettes share the same visual bounds in both weights.
    let (x, y) = match source {
        "puzzle-piece" => (-12, -8),
        "music-notes" => (-12, -4),
        "film-slate" => (-4, -8),
        "books" => (8, -4),
        "key" | "note-pencil" => (4, -4),
        "headphones" | "lock-key" => (0, -12),
        "briefcase" | "calendar" | "download-simple" | "upload-simple" => (0, -8),
        "camera" | "clipboard-text" | "house" | "palette" | "presentation-chart" | "star" => {
            (0, -4)
        }
        "graduation-cap" | "heart" | "plant" | "users-three" => (0, 8),
        _ => (0, 0),
    };
    let view_box = format!("{} {} 264 264", x - 4, y - 4);
    STANDARD.encode(svg.replace("0 0 256 256", &view_box))
}

pub(crate) fn css() -> String {
    let assets = include_str!("../../assets/vendor/phosphor/folder-icons.tsv")
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
    for group in GROUPS {
        let [light, middle, dark] = group.colors;
        for icon in group.icons {
            let (outline, filled) = assets[icon.source];
            writeln!(
                css,
                ".{}{{--folder-icon-mask:url(\"data:image/svg+xml;base64,{}\");\
                 --folder-color-mask:url(\"data:image/svg+xml;base64,{}\");\
                 --folder-icon-colors:linear-gradient(145deg,{light},{middle} 48%,{dark})}}",
                icon.class(),
                centered_mask(icon.source, outline),
                centered_mask(icon.source, filled),
            )
            .unwrap();
        }
    }
    css
}
