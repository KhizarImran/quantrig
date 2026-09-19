"""Long while the fast SMA is above the slow one, flat otherwise."""
from backtestingfx import Strategy

FAST, SLOW = 10, 30


class SmaCross(Strategy):
    def init(self):
        closes = [b.close for b in self._bars]
        self.long_signal = {}
        for i in range(SLOW - 1, len(closes)):
            window = closes[i - SLOW + 1 : i + 1]
            fast = sum(window[-FAST:]) / FAST
            slow = sum(window) / SLOW
            self.long_signal[self._bars[i].timestamp] = fast > slow

    def next(self):
        up = self.long_signal.get(self._bar.timestamp)
        if up is None:  # warmup
            return
        if up and not self.positions:
            self.buy(lot_size=0.1)
        elif not up and self.positions:
            self.close_all()
