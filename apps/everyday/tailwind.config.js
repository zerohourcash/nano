/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: ["class"],
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive) / <alpha-value>)",
          foreground: "hsl(var(--destructive-foreground) / <alpha-value>)",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        // ── MeshKeeper design tokens (design.md §2) ──
        brand: {
          50: '#EDEDF7',
          100: '#C9C9F0',
          600: '#5E629B',
          700: '#4C4F82',
        },
        ink: {
          900: '#303466',
          500: '#6B6E9E',
          300: '#9B9EC4',
        },
        app: '#ECECF3',
        surface: '#FFFFFF',
        accent: {
          DEFAULT: '#E0235B',
          hover: '#B91A49',
          foreground: '#FFFFFF',
        },
        teal: {
          DEFAULT: '#66C6BE',
          dark: '#2E8E86',
        },
        success: {
          DEFAULT: '#2E9E5B',
          bg: '#C8FCD2',
        },
        warning: {
          DEFAULT: '#A87C0F',
          bg: '#FBFCC8',
        },
        danger: {
          DEFAULT: '#D64545',
          bg: '#FAD8D1',
        },
        info: {
          bg: '#D8F2F0',
        },
      },
      fontFamily: {
        sans: ['"Segoe UI"', 'Roboto', 'system-ui', '-apple-system', 'sans-serif'],
        mono: ['ui-monospace', '"SFMono-Regular"', 'Consolas', 'monospace'],
      },
      borderRadius: {
        xl: "calc(var(--radius) + 4px)",
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
        xs: "calc(var(--radius) - 6px)",
        card: '16px',
        mini: '14px',
        modal: '18px',
      },
      boxShadow: {
        xs: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
        card: '0 1px 2px rgba(48,52,102,.06), 0 4px 16px rgba(48,52,102,.08)',
        hover: '0 4px 8px rgba(48,52,102,.10), 0 12px 32px rgba(48,52,102,.14)',
        modal: '0 24px 64px rgba(48,52,102,.28)',
      },
      backgroundImage: {
        'grad-brand': 'linear-gradient(135deg, #5E629B 0%, #7A7EC4 100%)',
        'grad-mesh': 'radial-gradient(circle at 20% 20%, #66C6BE33 0%, transparent 45%), radial-gradient(circle at 80% 75%, #C9C9F055 0%, transparent 50%), linear-gradient(0deg, #2E3160, #2E3160)',
      },
      maxWidth: {
        container: '1440px',
      },
      keyframes: {
        "accordion-down": {
          from: { height: "0" },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: "0" },
        },
        "caret-blink": {
          "0%,70%,100%": { opacity: "1" },
          "20%,50%": { opacity: "0" },
        },
        "skeleton-pulse": {
          "0%, 100%": { backgroundColor: "#EDEDF7" },
          "50%": { backgroundColor: "#E2E2F2" },
        },
        "badge-pop": {
          "0%": { transform: "scale(1)" },
          "50%": { transform: "scale(1.25)" },
          "100%": { transform: "scale(1)" },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
        "caret-blink": "caret-blink 1.25s ease-out infinite",
        "skeleton-pulse": "skeleton-pulse 1.4s ease-in-out infinite",
        "badge-pop": "badge-pop 0.3s ease-out",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
}
