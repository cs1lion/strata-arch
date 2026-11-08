//! ticket 20 模块搜索与聚焦的纯状态机。
//!
//! 只依赖布局模型，不依赖窗口：键位解释、相机转场与绘制留在 `main.rs`。
//! 判据见 `docs/tickets/arch-tool/20-module-search-focus.md`。
//!
//! 模型自身不读墙钟、不使用随机源、不注册定时器。

use crate::layout::{MapLayout, Point};
use crate::playback::CONTEXT_OPACITY;
use crate::view::Viewport;
use std::collections::{BTreeMap, BTreeSet};

/// 面板最多列出的命中条数；总数仍记在 `hit_count()`。
pub const MAX_VISIBLE_HITS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Closed,
    Input,
    Focused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    /// 0 = basename 完全相等，1 = basename 前缀，2 = basename 包含，3 = 仅路径包含。
    pub rank: u8,
}

fn basename(id: &str) -> &str {
    id.rsplit(['/', '\\']).next().unwrap_or(id)
}

/// 四级匹配排序；`query_lower` 与索引项均已小写化。
pub fn match_rank(query_lower: &str, id_lower: &str, basename_lower: &str) -> Option<u8> {
    if query_lower.is_empty() {
        return None;
    }
    if basename_lower == query_lower {
        Some(0)
    } else if basename_lower.starts_with(query_lower) {
        Some(1)
    } else if basename_lower.contains(query_lower) {
        Some(2)
    } else if id_lower.contains(query_lower) {
        Some(3)
    } else {
        None
    }
}

struct IndexEntry {
    id: String,
    id_lower: String,
    basename_lower: String,
}

/// 聚焦模块的一跳集合（上下游直接邻居）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusSet {
    pub focus: String,
    /// 指向聚焦模块的来源；自环不计入。
    pub upstream: BTreeSet<String>,
    /// 聚焦模块指向的目标；自环计入此处，保证全集合只计一次。
    pub downstream: BTreeSet<String>,
    /// `{focus} ∪ upstream ∪ downstream`，去重后的成员。
    pub members: BTreeSet<String>,
}

impl FocusSet {
    pub fn contains(&self, id: &str) -> bool {
        self.members.contains(id)
    }

    /// 焦点集合内 `1.0`，集合外沿用 P1 上下文下限 `0.68`。
    pub fn module_opacity(&self, id: &str) -> f32 {
        if self.contains(id) {
            1.0
        } else {
            CONTEXT_OPACITY
        }
    }

    /// 任一端点在焦点集合内的边保持 `1.0`，其余 `0.68`。
    pub fn edge_opacity(&self, source: &str, target: &str) -> f32 {
        if self.contains(source) || self.contains(target) {
            1.0
        } else {
            CONTEXT_OPACITY
        }
    }

    pub fn upstream_count(&self) -> usize {
        self.upstream.len()
    }

    pub fn downstream_count(&self) -> usize {
        self.downstream.len()
    }
}

/// 角落聚焦标签；计数与 `FocusSet` 的集合条目一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusLabel {
    pub module: String,
    pub upstream: usize,
    pub downstream: usize,
}

impl FocusLabel {
    pub fn text(&self) -> String {
        format!(
            "聚焦 {} · 上游 {} / 下游 {}",
            self.module, self.upstream, self.downstream
        )
    }
}

/// Enter 聚焦时交给调用方的相机请求；快照是聚焦前的视口。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRequest {
    pub target: Point,
    pub snapshot: Viewport,
}

/// 与平台无关的键位修饰键；`secondary` = Windows/Linux 的 Ctrl、macOS 的 Cmd。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub secondary: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKey {
    None,
    Open,
    Close,
    Confirm,
    Previous,
    Next,
    Backspace,
    Character(char),
}

/// 键位解释：把原始键名/字符收敛成搜索状态机可消费的动作。
pub fn interpret_key(key: &str, key_char: Option<&str>, modifiers: KeyModifiers) -> SearchKey {
    match key {
        "escape" => SearchKey::Close,
        "enter" => SearchKey::Confirm,
        "up" => SearchKey::Previous,
        "down" => SearchKey::Next,
        "backspace" => SearchKey::Backspace,
        "/" if !modifiers.secondary && !modifiers.alt => SearchKey::Open,
        "f" | "F" if modifiers.secondary => SearchKey::Open,
        _ => {
            if modifiers.secondary || modifiers.alt {
                return SearchKey::None;
            }
            key_char
                .and_then(|text| text.chars().next())
                .filter(|character| !character.is_control())
                .map(SearchKey::Character)
                .unwrap_or(SearchKey::None)
        }
    }
}

/// 状态机对外动作：调用方据此接线相机、阶段标记与重绘。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchAction {
    None,
    Opened,
    Closed,
    Focused(FocusRequest),
    Cleared(Viewport),
}

pub struct SearchState {
    mode: SearchMode,
    query: String,
    index: Vec<IndexEntry>,
    centers: BTreeMap<String, Point>,
    upstream: BTreeMap<String, BTreeSet<String>>,
    downstream: BTreeMap<String, BTreeSet<String>>,
    hits: Vec<SearchHit>,
    selected: usize,
    focus: Option<FocusSet>,
    snapshot: Option<Viewport>,
}

impl SearchState {
    /// 一次性建立匹配索引与邻接表；之后每次输入只扫描索引，不再碰布局。
    pub fn new(map: &MapLayout) -> Self {
        let index = map
            .nodes
            .iter()
            .map(|node| IndexEntry {
                id: node.id.clone(),
                id_lower: node.id.to_lowercase(),
                basename_lower: basename(&node.id).to_lowercase(),
            })
            .collect::<Vec<_>>();
        let centers = map
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.rect.center()))
            .collect::<BTreeMap<_, _>>();
        let mut upstream = map
            .nodes
            .iter()
            .map(|node| (node.id.clone(), BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut downstream = upstream.clone();
        for edge in &map.edges {
            if edge.source == edge.target {
                // 自环只计一次，归入下游（出边）。
                downstream
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
            } else {
                downstream
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
                upstream
                    .entry(edge.target.clone())
                    .or_default()
                    .insert(edge.source.clone());
            }
        }
        Self {
            mode: SearchMode::Closed,
            query: String::new(),
            index,
            centers,
            upstream,
            downstream,
            hits: Vec::new(),
            selected: 0,
            focus: None,
            snapshot: None,
        }
    }

    pub fn mode(&self) -> SearchMode {
        self.mode
    }

    pub fn is_open(&self) -> bool {
        self.mode == SearchMode::Input
    }

    pub fn is_focused(&self) -> bool {
        self.mode == SearchMode::Focused
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    pub fn hit_count(&self) -> usize {
        self.hits.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.hits.get(self.selected)
    }

    /// 打开输入态；聚焦态下忽略（需先 `Esc` 清除）。
    pub fn open(&mut self) -> bool {
        if self.mode == SearchMode::Focused {
            return false;
        }
        self.mode = SearchMode::Input;
        self.clear_query_state();
        true
    }

    /// 输入态 `Esc`：关闭面板并清空输入，不触碰相机与淡出。
    pub fn cancel_input(&mut self) -> bool {
        if self.mode != SearchMode::Input {
            return false;
        }
        self.mode = SearchMode::Closed;
        self.clear_query_state();
        true
    }

    /// 下钻等场景：整体回到初始态（含聚焦与快照）。
    pub fn reset(&mut self) {
        self.mode = SearchMode::Closed;
        self.focus = None;
        self.snapshot = None;
        self.clear_query_state();
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.recompute_hits();
    }

    pub fn push_char(&mut self, character: char) {
        self.query.push(character);
        self.recompute_hits();
    }

    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.recompute_hits();
        }
    }

    /// `↑/↓`：不循环，首尾各自停住。
    pub fn move_selection(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let last = (self.hits.len() - 1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// 可见窗口起点：选中项始终落在 `[start, start + MAX_VISIBLE_HITS)`。
    pub fn list_start(&self) -> usize {
        if self.hits.len() <= MAX_VISIBLE_HITS {
            return 0;
        }
        (self.selected + 1)
            .saturating_sub(MAX_VISIBLE_HITS)
            .min(self.hits.len() - MAX_VISIBLE_HITS)
    }

    pub fn visible_hits(&self) -> &[SearchHit] {
        let start = self.list_start();
        let end = (start + MAX_VISIBLE_HITS).min(self.hits.len());
        &self.hits[start..end]
    }

    /// 无命中固定为 `无匹配 · 0/0`；有命中显示 `前 6/共 N`。
    pub fn summary(&self) -> String {
        if self.hits.is_empty() {
            "无匹配 · 0/0".to_string()
        } else {
            format!("前 {}/共 {}", self.visible_hits().len(), self.hits.len())
        }
    }

    /// 输入态 `Enter`：进入聚焦态并返回相机目标与快照；无选中项时保持输入态。
    pub fn confirm(&mut self, viewport: Viewport) -> Option<FocusRequest> {
        if self.mode != SearchMode::Input {
            return None;
        }
        let hit = self.hits.get(self.selected)?.id.clone();
        let target = *self.centers.get(&hit)?;
        let upstream = self.upstream.get(&hit).cloned().unwrap_or_default();
        let downstream = self.downstream.get(&hit).cloned().unwrap_or_default();
        let mut members = upstream.clone();
        members.extend(downstream.iter().cloned());
        members.insert(hit.clone());
        self.focus = Some(FocusSet {
            focus: hit,
            upstream,
            downstream,
            members,
        });
        self.snapshot = Some(viewport);
        self.mode = SearchMode::Focused;
        // Enter 后面板关闭，query 同样不残留（判据 C1）。
        self.clear_query_state();
        Some(FocusRequest {
            target,
            snapshot: viewport,
        })
    }

    /// 聚焦态 `Esc`：清除淡出与标签，返回聚焦前视口供逐字段复原。
    pub fn clear_focus(&mut self) -> Option<Viewport> {
        if self.mode != SearchMode::Focused {
            return None;
        }
        let snapshot = self.snapshot.take();
        self.mode = SearchMode::Closed;
        self.focus = None;
        self.clear_query_state();
        snapshot
    }

    pub fn focus_set(&self) -> Option<&FocusSet> {
        if self.mode == SearchMode::Focused {
            self.focus.as_ref()
        } else {
            None
        }
    }

    pub fn focus_label(&self) -> Option<FocusLabel> {
        self.focus_set().map(|set| FocusLabel {
            module: set.focus.clone(),
            upstream: set.upstream_count(),
            downstream: set.downstream_count(),
        })
    }

    /// 单一入口：把键位动作映射到状态迁移；无效键保持原状并返回 `None`。
    pub fn handle(&mut self, key: SearchKey, viewport: Viewport) -> SearchAction {
        match (self.mode, key) {
            (SearchMode::Focused, SearchKey::Close) => self
                .clear_focus()
                .map(SearchAction::Cleared)
                .unwrap_or(SearchAction::None),
            (SearchMode::Focused, _) => SearchAction::None,
            (SearchMode::Closed, SearchKey::Open) => {
                self.open();
                SearchAction::Opened
            }
            (SearchMode::Closed, _) => SearchAction::None,
            (SearchMode::Input, SearchKey::Open) => SearchAction::None,
            (SearchMode::Input, SearchKey::Close) => {
                self.cancel_input();
                SearchAction::Closed
            }
            (SearchMode::Input, SearchKey::Confirm) => self
                .confirm(viewport)
                .map(SearchAction::Focused)
                .unwrap_or(SearchAction::None),
            (SearchMode::Input, SearchKey::Previous) => {
                self.move_selection(-1);
                SearchAction::None
            }
            (SearchMode::Input, SearchKey::Next) => {
                self.move_selection(1);
                SearchAction::None
            }
            (SearchMode::Input, SearchKey::Backspace) => {
                self.backspace();
                SearchAction::None
            }
            (SearchMode::Input, SearchKey::Character(character)) => {
                self.push_char(character);
                SearchAction::None
            }
            (SearchMode::Input, SearchKey::None) => SearchAction::None,
        }
    }

    pub fn module_opacity(&self, id: &str) -> f32 {
        self.focus_set()
            .map(|set| set.module_opacity(id))
            .unwrap_or(1.0)
    }

    pub fn edge_opacity(&self, source: &str, target: &str) -> f32 {
        self.focus_set()
            .map(|set| set.edge_opacity(source, target))
            .unwrap_or(1.0)
    }

    fn clear_query_state(&mut self) {
        self.query.clear();
        self.hits.clear();
        self.selected = 0;
    }

    fn recompute_hits(&mut self) {
        let query = self.query.to_lowercase();
        let mut hits = self
            .index
            .iter()
            .filter_map(|entry| {
                match_rank(&query, &entry.id_lower, &entry.basename_lower).map(|rank| SearchHit {
                    id: entry.id.clone(),
                    rank,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            left.rank
                .cmp(&right.rank)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.hits = hits;
        self.selected = 0;
    }
}
