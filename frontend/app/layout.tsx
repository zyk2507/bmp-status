import type { Metadata, Viewport } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "BMP Status · BGP Monitoring Protocol",
  description:
    "基于 Rust + bgpkit-parser 的 BMP 收集器：实时解析 BIRD / FRR / 路由器上报的 BGP Monitoring Protocol 消息并可视化。",
};

export const viewport: Viewport = {
  themeColor: "#05080f",
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}
