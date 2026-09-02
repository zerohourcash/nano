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
          50: '#F4F4F4',
          100: '#E1E1E1',
          600: '#191919',
          700: '#000000',
        },
        ink: {
          900: '#191919',
          500: '#727272',
          300: '#929292',
        },
        app: '#F8F8F8',
        surface: '#FFFFFF',
        accent: {
          DEFAULT: '#191919',
          hover: '#000000',
          foreground: '#FFFFFF',
        },
        teal: {
          DEFAULT: '#B49A78',
          dark: '#765C3D',
        },
        success: {
          DEFAULT: '#5C7D0B',
          bg: '#C8FCD2',
        },
        warning: {
          DEFAULT: '#C87000',
          bg: '#FBFCC8',
        },
        danger: {
          DEFAULT: '#C53929',
          bg: '#FAD8D1',
        },
        info: {
          bg: '#D8F2F0',
        },
      },
      fontFamily: {
        sans: ['"Helvetica Neue"', 'Helvetica', 'Arial', 'system-ui', 'sans-serif'],
        mono: ['ui-monospace', '"SFMono-Regular"', 'Consolas', 'monospace'],
      },
      borderRadius: {
        xl: "calc(var(--radius) + 4px)",
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
        xs: "calc(var(--radius) - 6px)",
        card: '2px',
        mini: '2px',
        modal: '4px',
      },
      boxShadow: {
        xs: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
        card: '0 1px 0 rgba(0,0,0,.04)',
        hover: '0 8px 28px rgba(0,0,0,.10)',
        modal: '0 24px 64px rgba(0,0,0,.20)',
      },
      backgroundImage: {
        'grad-brand': 'linear-gradient(135deg, #242424 0%, #000000 100%)',
        'grad-mesh': 'linear-gradient(135deg, #24211E 0%, #050505 100%)',
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
