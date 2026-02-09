# Scope Monitor Layout Architecture

## Layout System Design

Four preset layouts with shared components. Layouts are swappable at runtime via keybindings.

### Layout Presets

#### 1. Dashboard (Default)
```
┌─────────────────────────────────────────┐
│  Price Chart (60% width)  │  Stats     │
│                           │  Panel     │
│  [Candlesticks]           │  (40%)     │
│                           │  - Price   │
│                           │  - 24h Vol │
│                           │  - Liquidity│
│                           │  - Holders │
├─────────────────────────────────────────┤
│  Buy/Sell Gauge (50%) │ Volume (50%)  │
├─────────────────────────────────────────┤
│  Recent Transactions Feed               │
│  [timestamp] [addr] [amt] [type]        │
│  ...                                    │
└─────────────────────────────────────────┘
```
- Balanced view for general monitoring
- All widgets visible
- Good for: Initial exploration, overview

#### 2. Chart Focus
```
┌─────────────────────────────────────────┐
│                                         │
│                                         │
│         Full-Width Price Chart          │
│                                         │
│    [Candlesticks with volume overlay]   │
│                                         │
│                                         │
├─────────────────────────────────────────┤
│  Price: $X.XX  Vol: $XXM  [Alert: ▲]   │
└─────────────────────────────────────────┘
```
- Chart takes 85% of screen
- Minimal overlay stats
- Good for: Technical analysis, pattern watching

#### 3. Feed Mode
```
┌─────────────────────────────────────────┐
│  Live Price Ticker     [▲ $X.XX]       │
├─────────────────────────────────────────┤
│                                         │
│  Transaction Feed                       │
│  ─────────────────────────────────────  │
│  19:23:12  0x7a2f...  $12,450  BUY  ▲  │
│  19:23:08  0x9b1c...  $8,200   SELL ▼  │
│  19:22:45  0x3d4e...  $45,000  BUY  ▲  │
│  ...                                    │
│                                         │
│  [50 most recent transactions]          │
└─────────────────────────────────────────┘
```
- Transaction log prioritized
- Small persistent price ticker
- Good for: Tracking whale movements, MEV watching

#### 4. Compact
```
┌────────────────────────────┐
│ $X.XX  ▲5.2%  Vol: $12M   │
│ ─────────────────────────  │
│ [Mini chart: sparkline]   │
│ ─────────────────────────  │
│ Alerts: [Price > $Y]      │
└────────────────────────────┘
```
- Minimal footprint
- Sparkline instead of full chart
- Good for: Sidebar monitoring, low-distraction tracking

## Component Library

### Core Components

```rust
// Each component implements this trait
pub trait WidgetComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &MonitorState);
    fn handle_input(&mut self, key: KeyEvent) -> Option<Action>;
    fn name(&self) -> &'static str;
}

// Components
pub struct PriceChart {
    chart_type: ChartType,
    timeframe: Timeframe,
    scale: Scale,
    data: Vec<Candle>,
}

pub struct TransactionFeed {
    entries: Vec<Transaction>,
    max_entries: usize,
    auto_scroll: bool,
    scroll_offset: usize,
}

pub struct GaugePanel {
    buy_pressure: f64,
    sell_pressure: f64,
    volume_24h: f64,
}

pub struct StatsPanel {
    price_usd: f64,
    price_change_24h: f64,
    volume_24h: f64,
    liquidity_usd: f64,
    holder_count: u64,
}

pub struct AlertOverlay {
    active_alerts: Vec<Alert>,
    flash_until: Option<Instant>,
}
```

### Layout Manager

```rust
pub struct LayoutManager {
    current_layout: LayoutPreset,
    components: HashMap<String, Box<dyn WidgetComponent>>,
    config: MonitorConfig,
}

impl LayoutManager {
    pub fn switch_layout(&mut self, preset: LayoutPreset) {
        self.current_layout = preset;
        self.recalculate_areas();
    }
    
    pub fn toggle_widget(&mut self, widget: &str) {
        // Toggle visibility in current layout
    }
    
    fn calculate_areas(&self, frame: &Frame) -> LayoutAreas {
        match self.current_layout {
            LayoutPreset::Dashboard => self.dashboard_layout(frame.size()),
            LayoutPreset::ChartFocus => self.chart_focus_layout(frame.size()),
            LayoutPreset::Feed => self.feed_layout(frame.size()),
            LayoutPreset::Compact => self.compact_layout(frame.size()),
        }
    }
}
```

## Ratatui Implementation

### Dashboard Layout (ratatui constraints)

```rust
fn dashboard_layout(&self, area: Rect) -> LayoutAreas {
    // Split vertically: top (70%) / middle (15%) / bottom (15%)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(70),  // Chart + Stats
            Constraint::Percentage(15),  // Gauges
            Constraint::Percentage(15),  // Feed
        ])
        .split(area);
    
    // Top: horizontal split (chart 60% / stats 40%)
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(vertical[0]);
    
    // Middle: horizontal split (gauge / gauge)
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(vertical[1]);
    
    LayoutAreas {
        chart: top[0],
        stats: top[1],
        buy_pressure: middle[0],
        volume: middle[1],
        feed: vertical[2],
    }
}
```

### Responsive Breakpoints

```rust
impl LayoutManager {
    fn auto_select_layout(&self, size: Rect) -> LayoutPreset {
        match (size.width, size.height) {
            (w, h) if w < 80 || h < 24 => LayoutPreset::Compact,
            (w, _) if w < 120 => LayoutPreset::Feed,
            (_, h) if h < 30 => LayoutPreset::ChartFocus,
            _ => LayoutPreset::Dashboard,
        }
    }
}
```

## State Management

```rust
pub struct MonitorState {
    // Token being monitored
    pub token_address: String,
    pub chain: Chain,
    
    // Price data
    pub candles: Vec<Candle>,
    pub current_price: f64,
    pub price_change_24h: f64,
    
    // Market data
    pub volume_24h: f64,
    pub liquidity_usd: f64,
    pub holder_count: u64,
    pub buy_pressure: f64,
    pub sell_pressure: f64,
    
    // Transactions
    pub transactions: Vec<Transaction>,
    
    // Config
    pub config: MonitorConfig,
    
    // UI State
    pub active_alerts: Vec<Alert>,
    pub last_update: Instant,
    pub paused: bool,
}
```

## Implementation Phases

### Phase 1: Foundation
- [ ] Config loading (YAML)
- [ ] State management
- [ ] Basic terminal setup with ratatui

### Phase 2: Core Components
- [ ] PriceChart widget (candlesticks)
- [ ] TransactionFeed widget
- [ ] StatsPanel widget

### Phase 3: Layouts
- [ ] Dashboard layout
- [ ] Layout switching

### Phase 4: Polish
- [ ] Color schemes
- [ ] Alert system
- [ ] Keybindings
- [ ] Export mode

### Phase 5: Advanced
- [ ] Custom layouts
- [ ] Layout persistence
- [ ] Mouse support (optional)