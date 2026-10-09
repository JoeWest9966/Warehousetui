#!/usr/bin/env python3
"""nwt 真机首跑 pty 驱动:把面板当真人操作,逐幕留帧。

环境(子进程):
  NW_HOME=/opt/nwt-realrun/nw-home      真装配好的家(装配命令 Phase B 产物)
  XDG_CONFIG_HOME=/opt/nwt-realrun/config  隔离配置(初始无数据仓根)
  CWD=/opt/nwt-realrun/run              干净目录(无 catalog.json)
"""
import os, pty, sys, time, select, fcntl, termios, struct

COLS, ROWS = 110, 30
NWT = "/opt/Warehousetui/target/release/nwt"
LOGDIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "frames")

class Screen:
    """最小 ANSI 屏模:足够重建 ratatui 全屏应用的终局画面。
    跨 read 边界的不完整转义序列进 pending,下一段数据到了再解。"""
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.cx = self.cy = 0
        self.pending = ""

    def feed(self, data):
        data = self.pending + data
        self.pending = ""
        i, n = 0, len(data)
        while i < n:
            c = data[i]
            if c == "\x1b":
                if i + 1 >= n:  # 序列可能被 read 切断
                    self.pending = data[i:]
                    return
                if data[i + 1] == "[":
                    j = i + 2
                    while j < n and not ("@" <= data[j] <= "~"):
                        j += 1
                    if j >= n:  # 未收尾
                        self.pending = data[i:]
                        return
                    self.csi(data[i + 2 : j + 1])  # 含终止字节:csi 取 params[-1] 为命令
                    i = j + 1
                elif data[i + 1] == "]":  # OSC → BEL/ST
                    j = i + 2
                    while j < n and data[j] != "\x07" and data[j : j + 2] != "\x1b\\":
                        j += 1
                    if j >= n:
                        self.pending = data[i:]
                        return
                    i = j + 1
                else:
                    i += 2
            elif c == "\r":
                self.cx = 0; i += 1
            elif c == "\n":
                self.cy = min(self.cy + 1, self.rows - 1); i += 1
            elif c == "\b":
                self.cx = max(0, self.cx - 1); i += 1
            elif ord(c) < 32:
                i += 1
            else:
                if 0 <= self.cy < self.rows and 0 <= self.cx < self.cols:
                    self.grid[self.cy][self.cx] = c
                self.cx = min(self.cx + 1, self.cols - 1)
                i += 1

    def csi(self, params):
        if not params:
            self.cx = self.cy = 0; return
        cmd = params[-1]
        args = [int(x) for x in params[:-1].split(";") if x.isdigit()]
        if cmd in "Hf":
            self.cy = min(max((args[0] if args else 1) - 1, 0), self.rows - 1)
            self.cx = min(max((args[1] if len(args) > 1 else 1) - 1, 0), self.cols - 1)
        elif cmd == "J":
            m = args[0] if args else 0
            if m in (2, 3):
                self.grid = [[" "] * self.cols for _ in range(self.rows)]
            elif m == 0:
                for x in range(self.cx, self.cols): self.grid[self.cy][x] = " "
                for y in range(self.cy + 1, self.rows): self.grid[y] = [" "] * self.cols
        elif cmd == "K":
            m = args[0] if args else 0
            if m == 0:
                for x in range(self.cx, self.cols): self.grid[self.cy][x] = " "
            else:
                self.grid[self.cy] = [" "] * self.cols
        elif cmd == "A": self.cy = max(0, self.cy - (args[0] if args else 1))
        elif cmd == "B": self.cy = min(self.rows - 1, self.cy + (args[0] if args else 1))
        elif cmd == "C": self.cx = min(self.cols - 1, self.cx + (args[0] if args else 1))
        elif cmd == "D": self.cx = max(0, self.cx - (args[0] if args else 1))

    def text(self):
        return "\n".join("".join(row).rstrip() for row in self.grid)


def main():
    os.makedirs(LOGDIR, exist_ok=True)
    raw = open(f"{LOGDIR}/pty-raw.bin", "wb")
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["NW_HOME"] = "/opt/nwt-realrun/nw-home"
        os.environ["XDG_CONFIG_HOME"] = "/opt/nwt-realrun/config"
        os.environ.pop("XDG_DATA_HOME", None)
        os.chdir("/opt/nwt-realrun/run")
        os.execv(NWT, ["nwt"])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = Screen(COLS, ROWS)

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.1)
            if r:
                try:
                    data = os.read(fd, 1 << 16)
                except OSError:
                    return False
                if not data:
                    return False
                raw.write(data); raw.flush()
                screen.feed(data.decode("utf-8", "replace"))
        return True

    def snap(name, show=40):
        text = screen.text()
        with open(f"{LOGDIR}/frame-{name}.txt", "w") as f:
            f.write(text)
        print(f"\n========== FRAME {name} ==========")
        for line in text.splitlines()[:show]:
            print(line)

    def send(keys, wait=0.6):
        os.write(fd, keys.encode())
        pump(wait)

    print("== launch ==")
    pump(2.5)  # splash + bootstrap(version + 5 能力探针)
    snap("01-boot-no-dataroot")

    print("== settings: 录入数据仓根 /opt/Data-001 ==")
    send("k", 1.0)                  # 概览 → 设置(壳级环绕)
    pump(0.8)
    snap("02-settings-before")
    send("d", 0.5)                  # 数据仓根手填输入
    for ch in "/opt/Data-001":
        os.write(fd, ch.encode()); time.sleep(0.03)
    pump(0.5)
    snap("03-settings-typing")
    send("\r", 3.0)                 # Enter → apply + 真 ls
    snap("04-settings-root-set")

    print("== recipes: 真配方与组件 ==")
    send("k", 1.5)                  # 设置 → 配方(壳级切屏)
    snap("05-recipes")
    send("\x1b[B", 0.6)             # ↓ 选中 research-desk
    send("c", 1.0)                  # 建议命令
    snap("06-recipes-bake-suggest")

    print("== health: doctor ==")
    send("k", 1.5)                  # 配方 → 健康(壳级切屏)
    send("r", 8.0)                  # 真 doctor(探 provider,本机无 key)
    snap("07-health-doctor")

    print("== health: validate ==")
    send("m", 0.5)
    send("r", 6.0)
    snap("08-health-validate")

    print("== health: recheck ==")
    send("m", 0.5)
    send("r", 5.0)
    snap("09-health-recheck")

    print("== assemble: 选真锁定集 → 仪式 → 发射 ==")
    send("k", 1.5)                  # 健康 → 装配(壳级切屏)
    snap("10-assemble-picker")
    send("\r", 1.0)                 # Enter → 发射仪式
    snap("11-launch-ritual")
    send("\r", 45.0)                # Enter → 真 up(幂等重跑)
    snap("12-assemble-result", 40)

    print("== quit ==")
    send("q", 1.0)
    pump(1.0)
    try:
        os.close(fd)
    except OSError:
        pass
    os.waitpid(pid, 0)
    print("\nDONE")

if __name__ == "__main__":
    main()
