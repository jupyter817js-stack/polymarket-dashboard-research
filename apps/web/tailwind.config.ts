import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // 4단계 surface elevation — 깊이감
        bg: {
          DEFAULT:   "#0a0e14",   // root
          elevated:  "#0f141c",   // surface (cards)
          panel:     "#161d27",   // surface-2 (nested)
          soft:      "#1a212d",   // surface-3 (hover, subtle stripe)
        },
        // 2단계 line — divider 와 강한 line 구분
        line:    { DEFAULT: "#1f2935", strong: "#2a3543" },
        // 4단계 text hierarchy
        text:    {
          DEFAULT: "#e6edf3",
          muted:   "#9aa7b8",
          dim:     "#566373",
          faint:   "#3a4452",
        },
        // accent palette
        accent:  { DEFAULT: "#5bc0eb", 2: "#9f7aea", 3: "#22d3ee" },
        buy:     "#22c55e",
        sell:    "#ef4444",
        warn:    "#f59e0b",
        info:    "#3b82f6",
        chip:    "#1b2531",
      },
      borderRadius: {
        xs: "3px", sm: "5px", md: "7px", lg: "10px", xl: "14px", "2xl": "18px",
      },
      boxShadow: {
        "soft":   "0 1px 0 rgba(255,255,255,0.02) inset, 0 1px 2px rgba(0,0,0,0.3)",
        "card":   "0 1px 3px rgba(0,0,0,0.25), 0 1px 0 rgba(255,255,255,0.025) inset",
        "lift":   "0 4px 12px rgba(0,0,0,0.35), 0 1px 0 rgba(255,255,255,0.03) inset",
        "lift-2": "0 8px 24px rgba(0,0,0,0.4), 0 1px 0 rgba(255,255,255,0.04) inset",
        "glow":   "0 0 0 1px rgba(91,192,235,0.25), 0 4px 16px rgba(91,192,235,0.08)",
        "glow-buy":  "0 0 0 1px rgba(34,197,94,0.30), 0 6px 20px rgba(34,197,94,0.12)",
        "glow-sell": "0 0 0 1px rgba(239,68,68,0.30), 0 6px 20px rgba(239,68,68,0.12)",
        "inner-stroke": "inset 0 0 0 1px rgba(255,255,255,0.04)",
      },
      backdropBlur: {
        xs: "2px",
      },
      backgroundImage: {
        // Subtle radial sheen for hero surfaces
        "sheen":         "radial-gradient(circle at 0% 0%, rgba(91,192,235,0.08), transparent 45%)",
        "sheen-strong":  "radial-gradient(circle at 0% 0%, rgba(91,192,235,0.14), transparent 50%), radial-gradient(circle at 100% 100%, rgba(159,122,234,0.07), transparent 50%)",
        "sheen-buy":     "radial-gradient(circle at 0% 0%, rgba(34,197,94,0.10), transparent 50%)",
        "sheen-sell":    "radial-gradient(circle at 0% 0%, rgba(239,68,68,0.10), transparent 50%)",
        // Mesh gradient pattern for hero card backdrop
        "mesh":          "radial-gradient(ellipse 80% 60% at 20% 10%, rgba(91,192,235,0.10), transparent 60%), radial-gradient(ellipse 60% 40% at 80% 90%, rgba(159,122,234,0.08), transparent 60%)",
      },
      fontFamily: {
        sans: ['var(--font-sans)', '-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', '"Noto Sans KR"', 'sans-serif'],
        mono: ['var(--font-mono)', 'ui-monospace', 'SFMono-Regular', 'Menlo', 'monospace'],
      },
      letterSpacing: {
        tightish: "-0.01em",
        wider2:   "0.08em",
      },
      animation: {
        "fade-in":     "fade-in 200ms ease-out",
        "fade-in-up":  "fade-in-up 360ms cubic-bezier(0.16, 1, 0.3, 1)",
        "slide-up":    "slide-up 260ms cubic-bezier(0.16, 1, 0.3, 1)",
        "slide-down":  "slide-down 220ms cubic-bezier(0.16, 1, 0.3, 1)",
        "scale-in":    "scale-in 180ms cubic-bezier(0.16, 1, 0.3, 1)",
        "pulse-buy":   "pulse-buy 4s ease-out 1",
        "pulse-strong":"pulse-strong 4s ease-out 2",
        "pulse-flash": "pulse-flash 800ms ease-out 1",
        "shimmer":     "shimmer 1.6s linear infinite",
        "shine":       "shine 4s linear infinite",
      },
      keyframes: {
        "fade-in":     { from: { opacity: "0" }, to: { opacity: "1" } },
        "fade-in-up":  { from: { opacity: "0", transform: "translateY(6px)" }, to: { opacity: "1", transform: "translateY(0)" } },
        "slide-up":    { from: { opacity: "0", transform: "translateY(8px)" }, to: { opacity: "1", transform: "translateY(0)" } },
        "slide-down":  { from: { opacity: "0", transform: "translateY(-6px)" }, to: { opacity: "1", transform: "translateY(0)" } },
        "scale-in":    { from: { opacity: "0", transform: "scale(0.96)" }, to: { opacity: "1", transform: "scale(1)" } },
        "pulse-buy":   {
          "0%":   { backgroundColor: "rgba(34,197,94,0.32)", boxShadow: "0 0 0 0 rgba(34,197,94,0.55)" },
          "12%":  { backgroundColor: "rgba(34,197,94,0.28)", boxShadow: "0 0 0 12px rgba(34,197,94,0.0)" },
          "55%":  { backgroundColor: "rgba(34,197,94,0.10)" },
          "100%": { backgroundColor: "rgba(34,197,94,0.0)" },
        },
        "pulse-strong": {
          "0%, 100%": { boxShadow: "0 0 0 0 rgba(34,197,94,0.45)" },
          "50%":      { boxShadow: "0 0 0 6px rgba(34,197,94,0.0)" },
        },
        "pulse-flash": {
          "0%":   { backgroundColor: "rgba(91,192,235,0.0)" },
          "30%":  { backgroundColor: "rgba(91,192,235,0.12)" },
          "100%": { backgroundColor: "rgba(91,192,235,0.0)" },
        },
        "shimmer": {
          "0%":   { backgroundPosition: "-200% 0" },
          "100%": { backgroundPosition: "200% 0" },
        },
        "shine": {
          "0%":   { backgroundPosition: "200% 0" },
          "100%": { backgroundPosition: "-200% 0" },
        },
      },
    },
  },
  plugins: [],
};
export default config;
