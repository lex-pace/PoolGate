## UI定位

PoolGated的常驻桌面托盘应用。
设计目标：

- 接近 iOS 26 Liquid Glass 风格
- 接近 macOS Tahoe 半透明材质
- 支持 Windows Fluent Acrylic 风格
- 高级、轻量、信息密度适中

核心体验：

点击系统托盘 Icon

↓

展开固定尺寸 Glass Panel

↓

通过底部导航切换 Dashboard / Gateway / Token / Settings

---

# 1. 托盘窗口基础规范

## 1.1 窗口尺寸

固定尺寸：
Width: 380px
Height: 720px

禁止：

- 自适应宽高
- 用户拖拽缩放

原因：

## 保证 macOS Menu Bar 和 Windows Tray 一致体验。

## 1.2 窗口圆角

border-radius: 28px;

---

## 1.3 窗口背景

## Light Mode:
background:rgba(245,248,255,0.65)
Dark Mode:rgba(20,25,35,0.75)

## 1.4 毛玻璃效果

必须开启：

CSS:

```css
backdrop-filter:blur(45px) saturate(180%);
```

Electron:
BrowserWindow:
transparent:true
vibrancy:"under-window"

## 1.5 外边框

```css
border:1px solid rgba(255,255,255,0.45);
```

## 1.6 阴影

```css
box-shadow:0 20px 60px rgba(0,0,0,.18);
```

# 2. 整体布局

窗口结构：

┌──────────────────┐

 Header

 Dashboard Content

 Footer Navigation

└──────────────────┘
内部 Padding:16px

# 3. Header组件

高度: 72px
布局：
Logo
PoolGate
本地 Agent 网关
状态
时间Segment

## Logo

尺寸：40x40px
圆角：12px
效果：玻璃浮层。

## 产品名称

字体：
macOS:SF Pro Display

Windows:Segoe UI Variable

字号：20px
字重：600

## Subtitle

本地 Agent 网关
字号：12px
颜色：#8E8E93

## 在线状态

在线：● 在线
颜色：#34C759
圆点：8px

# 4. 时间选择器

位置：Header右侧
尺寸：220 x 36px
背景：background:rgba(255,255,255,.35)
圆角：18px

Active状态:

```css
background:white;
box-shadow:0 2px 8px rgba(0,0,0,.12);
```



# 5. Card系统

所有模块统一 Card。

- 包括：
- 路由拓扑
- Token统计
- 模型使用
- 活动
- 趋势
## Card

宽度：348px
圆角：22px
背景：rgba(255,255,255,0.35)
边框：1px solid rgba(255,255,255,.4)
Padding:16px

# 6. Dashboard页面
## 6.1 实时路由拓扑
标题：实时路由拓扑
右侧：127.0.0.1:9800
## Route Item
高度：52px
结构：

Icon

Label

Value

Status

例如：
Gateway          127.0.0.1:9800

Protocol · 4     OpenAI Compatible

Route Pool ·3    ChatGPT号码池

Provider ·7      Google Antigravity

## Item样式
背景：rgba(255,255,255,.45)
圆角：14px
间距：8px
# 7. Resource Dashboard
三个指标卡。
布局：
[资源可用率]

[可用路由池]

[今日Tokens]

Mini Card:
尺寸:100x90px
圆角：18px
数字:
字号:26px
颜色：#1D1D1F

# 8. 环形进度组件
用于：
- 资源率
- Token占比
参数：
Stroke:6px
Diameter:44px

颜色:

Blue:#007AFF
Green:#34C759

# 9. Chart区域
请求趋势图

高度：180px
背景：透明
Line:
#007AFF
2px

Grid:
opacity:0.08

# 10. Token页面
页面顶部：

TODAY TOKENS

240,408

$0.0819

## Token数字
字号：42px

字体：SF Rounded

颜色：#007AFF

# 11. Token模型列表
结构：

模型使用

mimo-v2.5-pro        240.4K       100%

Progress Bar:

高度:4px

圆角:4px

颜色:#007AFF

# 12. Empty State
无数据状态：

禁止空白。

显示：


      □ 暂无活动记录

Icon:48x48

透明度：40%

# 13. Footer Navigation
固定底部。

高度：72px

布局：
首页   网关  Token  设置

## Navigation Item

尺寸：70x48

Active:
背景：rgba(255,255,255,.8)

圆角：14px

阴影：0 2px 8px rgba(0,0,0,.1)

# 14. 动画规范
## 页面切换

Duration:250ms

Easing:ease-out

## Card进入动画

初始：opacity:0 translateY(12px)

结束：opacity:1 translateY(0)

# 15. 颜色设计系统
## Primary
#007AFF
## Success
#34C759
## Warning
#FF9500
## Danger
#FF3B30
## Primary Text
#1D1D1F
## Secondary Text
#86868B

# 16. 字体规范
macOS:
SF Pro Display
SF Pro Text
SF Rounded

Windows:
Segoe UI Variable

中文：
PingFang SC
Microsoft YaHei

# 17. CSS变量
```css
:root {
--glass-bg:
rgba(255,255,255,.35);
--glass-border:
rgba(255,255,255,.45);
--radius-window:
28px;
--radius-card:
22px;
--primary:
#007AFF;
--success:
#34C759;
}
```

# 19. 验收标准

必须达到：
✅ 窗口380×720
✅ 圆角28px
✅ Liquid Glass毛玻璃
✅ Card半透明
✅ Header固定
✅ Footer固定
✅ 页面切换动画
✅ Retina高清
✅ Light/Dark适配

# 20. 最终视觉目标

参考：
Apple iOS 26 Control Center
macOS Tahoe Menu Bar Apps
Raycast
CleanShot X
Lunar
最终效果：
一个高级 AI Agent 本地控制中心。