from PIL import Image, ImageDraw

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

def rrect(box, radius, fill):
    d.rounded_rectangle(box, radius=radius, fill=fill)

# 背景
rrect((40, 40, S-40, S-40), 200, (30, 36, 48, 255))

# 选区框
x0, y0, x1, y1 = 220, 280, 804, 744
w = 44
d.rectangle((x0, y0, x1, y0+w), fill=(255, 155, 46, 255))
d.rectangle((x0, y1-w, x1, y1), fill=(255, 155, 46, 255))
d.rectangle((x0, y0, x0+w, y1), fill=(255, 155, 46, 255))
d.rectangle((x1-w, y0, x1, y1), fill=(255, 155, 46, 255))

# 四角手柄
h = 60
for cx, cy in ((x0, y0), (x1, y0), (x0, y1), (x1, y1)):
    d.rounded_rectangle((cx-h//2, cy-h//2, cx+h//2, cy+h//2), radius=14, fill=(255, 255, 255, 255))

# 中心十字（半透明）
c = 512
d.line((c-120, c, c+120, c), fill=(255, 255, 255, 120), width=10)
d.line((c, c-120, c, c+120), fill=(255, 255, 255, 120), width=10)

sizes = [(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (24, 24), (16, 16)]
base = img.resize((256, 256), Image.LANCZOS)
base.save("assets/app.ico", format="ICO", sizes=sizes)
print("icon ok")