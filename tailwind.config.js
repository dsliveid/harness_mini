/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // 深色主题配色（参照 ZCode 风格：黑灰为主 + 品牌色点缀）
        panel: "#18181b",
        panel2: "#1f1f23",
        panel3: "#27272a",
        edge: "#3f3f46",
        ink: "#e4e4e7",
        inkdim: "#a1a1aa",
        accent: "#3b82f6",
      },
      fontFamily: {
        mono: ["Consolas", "JetBrains Mono", "monospace"],
      },
    },
  },
  plugins: [],
};
