import type { BucketView } from "@/lib/types";
import { formatClock, formatNumber } from "@/lib/format";

const W = 760;
const H = 248;
const PAD = { left: 42, right: 14, top: 14, bottom: 28 };

function niceMax(value: number): number {
  if (value <= 4) return 4;
  const power = 10 ** Math.floor(Math.log10(value));
  const normalized = value / power;
  const step = normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10;
  return step * power;
}

export function TrafficChart({ buckets }: { buckets: BucketView[] }) {
  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;
  const n = buckets.length;
  const peak = buckets.reduce((acc, bucket) => Math.max(acc, bucket.messages), 0);
  const yMax = niceMax(peak);
  const current = buckets[n - 1]?.messages ?? 0;
  const announcements = buckets.reduce((acc, bucket) => acc + bucket.announcements, 0);
  const withdrawals = buckets.reduce((acc, bucket) => acc + bucket.withdrawals, 0);

  const x = (index: number) => PAD.left + (n <= 1 ? plotW / 2 : (index * plotW) / (n - 1));
  const y = (value: number) => PAD.top + plotH - (value / yMax) * plotH;
  const bottom = PAD.top + plotH;
  const barWidth = Math.max(1.4, (plotW / Math.max(n, 1)) * 0.55);

  const linePath = buckets
    .map((bucket, index) => `${index === 0 ? "M" : "L"}${x(index).toFixed(1)},${y(bucket.messages).toFixed(1)}`)
    .join(" ");
  const areaPath = `${linePath} L${x(Math.max(n - 1, 0)).toFixed(1)},${bottom} L${x(0).toFixed(1)},${bottom} Z`;

  const gridLines = [0, 0.25, 0.5, 0.75, 1];
  const xLabelIndexes = n > 1 ? [0, Math.floor(n * 0.25), Math.floor(n * 0.5), Math.floor(n * 0.75), n - 1] : [0];

  return (
    <div className="traffic-chart">
      <div className="chart-summary">
        <div>
          <span>当前</span>
          <strong>{formatNumber(current)}<em>/s</em></strong>
        </div>
        <div>
          <span>窗口峰值</span>
          <strong>{formatNumber(peak)}<em>/s</em></strong>
        </div>
        <div>
          <span>累计更新</span>
          <strong className="up">↑{formatNumber(announcements)}</strong>
          <strong className="down">↓{formatNumber(withdrawals)}</strong>
        </div>
      </div>

      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="最近 120 秒 BMP 消息流量">
        <defs>
          <linearGradient id="bmp-traffic-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#818cf8" stopOpacity="0.32" />
            <stop offset="100%" stopColor="#818cf8" stopOpacity="0.01" />
          </linearGradient>
          <linearGradient id="bmp-traffic-line" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="#22d3ee" />
            <stop offset="100%" stopColor="#a5b4fc" />
          </linearGradient>
        </defs>

        {gridLines.map((fraction) => {
          const value = yMax * fraction;
          const gy = y(value);
          return (
            <g key={fraction}>
              <line
                x1={PAD.left}
                x2={W - PAD.right}
                y1={gy}
                y2={gy}
                stroke="rgba(148,163,184,0.12)"
                strokeDasharray={fraction === 0 ? undefined : "3 5"}
              />
              <text x={PAD.left - 8} y={gy + 3.5} textAnchor="end" className="axis-label">
                {formatNumber(value)}
              </text>
            </g>
          );
        })}

        {buckets.map((bucket, index) => {
          const bx = x(index) - barWidth / 2;
          const annH = (bucket.announcements / yMax) * plotH;
          const wdH = (bucket.withdrawals / yMax) * plotH;
          return (
            <g key={bucket.t}>
              {wdH > 0 && (
                <rect
                  x={bx}
                  y={bottom - wdH}
                  width={barWidth}
                  height={wdH}
                  fill="#fb7185"
                  opacity="0.55"
                  rx="1"
                />
              )}
              {annH > 0 && (
                <rect
                  x={bx}
                  y={bottom - wdH - annH}
                  width={barWidth}
                  height={annH}
                  fill="#22d3ee"
                  opacity="0.5"
                  rx="1"
                />
              )}
            </g>
          );
        })}

        {n > 0 && <path d={areaPath} fill="url(#bmp-traffic-fill)" />}
        {n > 0 && (
          <path
            d={linePath}
            fill="none"
            stroke="url(#bmp-traffic-line)"
            strokeWidth="2"
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        )}

        {xLabelIndexes.map((index) => {
          const bucket = buckets[index];
          return (
            <text
              key={`${index}-${bucket?.t ?? 0}`}
              x={x(index)}
              y={H - 8}
              textAnchor={index === 0 ? "start" : index === n - 1 ? "end" : "middle"}
              className="axis-label"
            >
              {bucket ? formatClock(bucket.t) : ""}
            </text>
          );
        })}

        {buckets.map((bucket, index) => (
          <rect
            key={`hit-${bucket.t}`}
            x={x(index) - plotW / Math.max(n, 1) / 2}
            y={PAD.top}
            width={plotW / Math.max(n, 1)}
            height={plotH}
            fill="transparent"
          >
            <title>
              {`${formatClock(bucket.t)} · ${bucket.messages} 消息 · ↑${bucket.announcements} · ↓${bucket.withdrawals}${bucket.errors ? ` · ${bucket.errors} 错误` : ""}`}
            </title>
          </rect>
        ))}
      </svg>

      <div className="chart-legend">
        <span><i className="legend-line" /> 消息总数</span>
        <span><i className="legend-bar cyan" /> 通告</span>
        <span><i className="legend-bar rose" /> 撤销</span>
      </div>
    </div>
  );
}
