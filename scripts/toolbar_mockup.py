"""Render toolbar mockups for review (pure-icon design)."""

from PIL import Image, ImageDraw, ImageFont, ImageFilter
import math

S = 4  # supersample factor; final = S/2 (2x display)
OUT_SCALE = 2
FONT_CJK = "C:/Windows/Fonts/msyh.ttc"
FONT_UI = "C:/Windows/Fonts/segoeui.ttf"

ACCENT = (255, 155, 46)
PANEL = (27, 27, 32)
PANEL_BORDER = (58, 58, 66)
ICON = (232, 232, 236)
ICON_DIM = (110, 110, 118)
HOVER = (46, 46, 54)
TEXT = (225, 225, 230)
TEXT_DIM = (150, 150, 158)


def font(path, size):
    try:
        return ImageFont.truetype(path, size)
    except OSError:
        return ImageFont.load_default()


class Scaled:
    def __init__(self, w, h, bg):
        self.w, self.h = w, h
        self.img = Image.new("RGBA", (w * S, h * S), bg)
        self.d = ImageDraw.Draw(self.img)

    def p(self, xy):
        return tuple(int(v * S) for v in xy)

    def rect(self, box, fill=None, outline=None, width=1, radius=0):
        box = self.p(box)
        w = max(1, int(width * S))
        if radius:
            self.d.rounded_rectangle(box, radius=int(radius * S), fill=fill,
                                     outline=outline, width=w)
        else:
            self.d.rectangle(box, fill=fill, outline=outline, width=w)

    def line(self, pts, fill, width=1):
        self.d.line([self.p(p) for p in pts], fill=fill, width=max(1, int(width * S)),
                    joint="curve")

    def ellipse(self, box, fill=None, outline=None, width=1):
        self.d.ellipse(self.p(box), fill=fill, outline=outline,
                       width=max(1, int(width * S)))

    def polygon(self, pts, fill=None, outline=None, width=1):
        self.d.polygon([self.p(p) for p in pts], fill=fill, outline=outline,
                       width=max(1, int(width * S)))

    def text(self, xy, s, size=14, fill=TEXT, anchor="la", bold=False):
        f = font(FONT_CJK, int(size * S))
        self.d.text(self.p(xy), s, font=f, fill=fill, anchor=anchor)

    def save(self, path):
        final = self.img.resize((self.w * OUT_SCALE, self.h * OUT_SCALE), Image.LANCZOS)
        final.convert("RGB").save(path)
        print("saved", path, final.size)


# ---------------- icons (1x coordinates, 18x18 box) ----------------

def icon_select(c, x, y, color):
    c.polygon([(x + 3, y + 1), (x + 3, y + 15), (x + 7, y + 11),
               (x + 10, y + 17), (x + 13, y + 15), (x + 10, y + 9), (x + 16, y + 8)],
              fill=None, outline=color, width=1.6)
    c.line([(x + 3, y + 1), (x + 3, y + 15)], color, 1.6)


def icon_pen(c, x, y, color):
    c.line([(x + 4, y + 14), (x + 13, y + 4)], color, 2.0)
    c.polygon([(x + 3, y + 16), (x + 4, y + 12), (x + 7, y + 15)], outline=color, width=1.2)
    c.line([(x + 12, y + 3), (x + 15, y + 6)], color, 2.0)


def icon_line(c, x, y, color):
    c.line([(x + 4, y + 14), (x + 14, y + 4)], color, 1.8)
    c.ellipse((x + 2, y + 12, x + 6, y + 16), fill=color)
    c.ellipse((x + 12, y + 2, x + 16, y + 6), fill=color)


def icon_arrow(c, x, y, color):
    c.line([(x + 3, y + 15), (x + 14, y + 4)], color, 1.8)
    c.line([(x + 8, y + 3), (x + 15, y + 3), (x + 15, y + 10)], color, 1.8)


def icon_rect(c, x, y, color):
    c.rect((x + 3, y + 3, x + 15, y + 15), outline=color, width=1.7)


def icon_ellipse(c, x, y, color):
    c.ellipse((x + 2, y + 3, x + 16, y + 15), outline=color, width=1.7)


def icon_highlight(c, x, y, color):
    # 粗笔触 + 斜切笔头，和铅笔区分开
    c.line([(x + 5, y + 13), (x + 14, y + 4)], (255, 214, 92, 160), 6.0)
    c.polygon([(x + 2, y + 16), (x + 3, y + 11), (x + 7, y + 14)],
              fill=(255, 214, 92, 230))
    c.line([(x + 4, y + 15), (x + 13, y + 6)], color, 1.0)


def icon_text(c, x, y, color):
    c.line([(x + 3, y + 3), (x + 15, y + 3)], color, 1.8)
    c.line([(x + 9, y + 3), (x + 9, y + 16)], color, 1.8)


def icon_mosaic(c, x, y, color):
    for row in range(3):
        for col in range(3):
            bx = x + 2 + col * 5
            by = y + 2 + row * 5
            if (row + col) % 2 == 0:
                c.rect((bx, by, bx + 4, by + 4), fill=color)
            else:
                c.rect((bx, by, bx + 4, by + 4), outline=color, width=1.0)


def icon_step(c, x, y, color):
    c.ellipse((x + 2, y + 2, x + 16, y + 16), outline=color, width=1.6)
    c.text((x + 9, y + 9), "1", size=8, fill=color, anchor="mm")


def icon_undo(c, x, y, color, flip=False):
    if flip:
        pts = [(x + 15, y + 12), (x + 9, y + 12), (x + 6, y + 9), (x + 9, y + 6), (x + 15, y + 6)]
        c.line(pts, color, 1.6)
        c.line([(x + 15, y + 6), (x + 11, y + 3)], color, 1.6)
        c.line([(x + 15, y + 6), (x + 11, y + 9)], color, 1.6)
    else:
        pts = [(x + 3, y + 12), (x + 9, y + 12), (x + 12, y + 9), (x + 9, y + 6), (x + 3, y + 6)]
        c.line(pts, color, 1.6)
        c.line([(x + 3, y + 6), (x + 7, y + 3)], color, 1.6)
        c.line([(x + 3, y + 6), (x + 7, y + 9)], color, 1.6)


def icon_copy(c, x, y, color):
    c.rect((x + 4, y + 4, x + 13, y + 14), outline=color, width=1.5)
    c.rect((x + 7, y + 2, x + 16, y + 11), outline=color, width=1.5)


def icon_save(c, x, y, color):
    c.line([(x + 9, y + 2), (x + 9, y + 11)], color, 1.7)
    c.line([(x + 5, y + 8), (x + 9, y + 12), (x + 13, y + 8)], color, 1.7)
    c.line([(x + 3, y + 15), (x + 15, y + 15)], color, 1.7)


def icon_close(c, x, y, color):
    c.line([(x + 4, y + 4), (x + 14, y + 14)], color, 1.8)
    c.line([(x + 14, y + 4), (x + 4, y + 14)], color, 1.8)


def icon_minus(c, x, y, color):
    c.line([(x + 4, y + 9), (x + 14, y + 9)], color, 1.8)


def icon_plus(c, x, y, color):
    c.line([(x + 4, y + 9), (x + 14, y + 9)], color, 1.8)
    c.line([(x + 9, y + 4), (x + 9, y + 14)], color, 1.8)


TOOLS = [icon_select, icon_pen, icon_line, icon_arrow, icon_rect, icon_ellipse,
         icon_highlight, icon_text, icon_mosaic, icon_step]
COLORS = [(255, 59, 48), (255, 149, 0), (255, 204, 0), (52, 199, 89),
          (0, 122, 255), (175, 82, 222), (255, 255, 255), (0, 0, 0)]


def draw_panel(c, x, y, scale=1.0, active_tool=3, hover_tool=None):
    cell = 30 * scale
    gap = 4 * scale
    pad = 8 * scale
    width = 390 * scale
    height = 118 * scale

    # shadow + panel
    c.rect((x - 4, y - 2, x + width + 6, y + height + 8), fill=(0, 0, 0, 90), radius=16 * scale)
    c.rect((x, y, x + width, y + height), fill=(*PANEL, 242),
           outline=PANEL_BORDER, width=1, radius=12 * scale)

    # row 1: tools (centered)
    row_w = 10 * cell + 9 * gap
    tx = x + (width - row_w) / 2
    ty = y + pad
    for i, icon in enumerate(TOOLS):
        bx = tx + i * (cell + gap)
        if i == active_tool:
            c.rect((bx, ty, bx + cell, ty + cell), fill=ACCENT, radius=8 * scale)
            icon(c, bx + (cell - 18) / 2, ty + (cell - 18) / 2, (26, 26, 26))
        else:
            if hover_tool == i:
                c.rect((bx, ty, bx + cell, ty + cell), fill=HOVER, radius=8 * scale)
            icon(c, bx + (cell - 18) / 2, ty + (cell - 18) / 2, ICON)

    # row 2: colors + width
    row_y = ty + cell + 6 * scale
    sx = x + pad
    for i, color in enumerate(COLORS):
        cx = sx + i * (cell + gap) + cell / 2
        cy = row_y + cell / 2
        r = 10 * scale
        c.ellipse((cx - r, cy - r, cx + r, cy + r), fill=color,
                  outline=(0, 0, 0, 90), width=1)
        if i == 0:
            c.ellipse((cx - r - 2 * scale, cy - r - 2 * scale, cx + r + 2 * scale, cy + r + 2 * scale),
                      outline=(255, 255, 255, 230), width=1.6 * scale)

    wx = sx + 8 * (cell + gap) + 6 * scale
    c.line([(wx, row_y + 2), (wx, row_y + cell - 2)], (255, 255, 255, 26), 1)
    wx += 6 * scale
    c.rect((wx, row_y, wx + 26 * scale, row_y + cell), fill=HOVER, radius=7 * scale)
    icon_minus(c, wx + 4 * scale, row_y + 6 * scale, ICON)
    wx += 26 * scale + 4 * scale
    c.text((wx + 17 * scale, row_y + cell / 2), "4", size=13 * scale,
           fill=ICON, anchor="mm")
    c.text((wx + 24 * scale, row_y + cell / 2 - 4 * scale), "px", size=8 * scale,
           fill=TEXT_DIM, anchor="lm")
    wx += 40 * scale
    c.rect((wx, row_y, wx + 26 * scale, row_y + cell), fill=HOVER, radius=7 * scale)
    icon_plus(c, wx + 4 * scale, row_y + 6 * scale, ICON)

    # row 3: actions (right aligned)
    row_y += cell + 6 * scale
    actions_x = x + width - pad - (5 * cell + 4 * gap)
    ax = actions_x
    icon_undo(c, ax + (cell - 18) / 2, row_y + (cell - 18) / 2, ICON)
    ax += cell + gap
    icon_undo(c, ax + (cell - 18) / 2, row_y + (cell - 18) / 2, ICON_DIM, flip=True)
    ax += cell + gap
    c.line([(ax - gap / 2, row_y + 4), (ax - gap / 2, row_y + cell - 4)],
           (255, 255, 255, 26), 1)
    icon_copy(c, ax + (cell - 18) / 2, row_y + (cell - 18) / 2, ICON)
    ax += cell + gap
    icon_save(c, ax + (cell - 18) / 2, row_y + (cell - 18) / 2, ICON)
    ax += cell + gap
    c.rect((ax, row_y, ax + cell, row_y + cell), fill=(120, 40, 40, 0), radius=7 * scale)
    icon_close(c, ax + (cell - 18) / 2, row_y + (cell - 18) / 2, (255, 120, 110))


# ---------------- mockup 1: close-up ----------------

def mockup_closeup(path):
    c = Scaled(900, 470, (16, 16, 20, 255))
    c.text((30, 18), "window-cap 工具栏 · 纯图标方案（3 行紧凑）", 18)
    c.text((30, 48), "深色面板 + 当前工具 accent 填充；hover 提亮、禁用变暗；颜色选中加白环",
           12, TEXT_DIM)

    draw_panel(c, 30, 78, scale=1.0, active_tool=3, hover_tool=6)
    c.text((440, 78), "① 10 个工具：矢量线框图标，18px", 13)
    c.text((440, 102), "② 当前工具：accent 填充 + 深色图标", 13)
    c.text((440, 126), "③ hover：底色提亮一档", 13)
    c.text((440, 150), "④ 8 色圆形色块，选中加白环", 13)
    c.text((440, 174), "⑤ 线宽：− / 数值 / +", 13)
    c.text((440, 198), "⑥ 撤销、重做、复制、保存、关闭", 13)

    # state strip
    c.text((30, 230), "状态示例", 14)
    labels = ["普通", "hover", "选中", "禁用"]
    cells = [(24, ICON), (24, ICON), (24, (26, 26, 26)), (24, ICON_DIM)]
    x = 30
    for i, (cell, color) in enumerate(cells):
        y = 260
        if i == 1:
            c.rect((x, y, x + 44, y + 44), fill=HOVER, radius=10)
        elif i == 2:
            c.rect((x, y, x + 44, y + 44), fill=ACCENT, radius=10)
        if i == 3:
            c.rect((x, y, x + 44, y + 44), fill=(30, 30, 34, 160), radius=10)
        icon_arrow(c, x + 13, y + 13, color)
        c.text((x + 22, y + 52), labels[i], 12, TEXT_DIM, anchor="ma")
        x += 90

    c.text((30, 350), "后期再加：纯图标 tooltip（鼠标悬停显示工具名 + 快捷键）", 12, TEXT_DIM)
    c.text((30, 380), "尺寸：1x 390×118；布局按 DPI scale 等比缩放（150% / 200% 不模糊）",
           12, TEXT_DIM)
    c.text((30, 410), "配色：面板 #1B1B20 / 边框 #3A3A42 / accent #FF9B2E / 图标 #E8E8EC",
           12, TEXT_DIM)
    c.save(path)


# ---------------- mockup 2: in context ----------------

def mockup_context(path):
    c = Scaled(900, 470, (18, 16, 22, 255))
    # fake dimmed desktop
    c.rect((0, 0, 900, 470), fill=(20, 18, 24, 255))
    c.ellipse((520, -80, 980, 260), fill=(60, 30, 46, 255))
    c.ellipse((40, 280, 420, 620), fill=(28, 36, 44, 255))
    c.rect((520, 120, 760, 300), fill=(34, 30, 40, 255), radius=8)
    # bright selection
    c.rect((90, 70, 790, 400), fill=(96, 70, 84, 255), outline=(255, 255, 255, 30), width=1)
    c.rect((90, 70, 790, 400), outline=ACCENT, width=2)
    c.text((100, 80), "选区内容（亮）", 12, (240, 230, 235))
    for i in range(6):
        c.rect((120 + i * 105, 120, 200 + i * 105, 300), fill=(120 + i * 8, 84 + i * 4, 96, 255), radius=4)

    # panel
    draw_panel(c, 370, 260, scale=1.0, active_tool=3, hover_tool=None)

    # selection handles
    for hx, hy in [(90, 70), (440, 70), (790, 70), (90, 400), (440, 400), (790, 400)]:
        c.rect((hx - 3, hy - 3, hx + 3, hy + 3), fill=(255, 255, 255, 255), outline=ACCENT, width=1)

    c.text((30, 430), "实际使用位置：工具栏固定在选区右下角，整体随 DPI 缩放", 12, TEXT_DIM)
    c.save(path)


if __name__ == "__main__":
    mockup_closeup("target/toolbar-mockup.png")
    mockup_context("target/toolbar-context.png")
