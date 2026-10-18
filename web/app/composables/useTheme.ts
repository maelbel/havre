// Dark by default, light on demand. Remembered in this browser only.
export function useTheme() {
  const theme = useState<'dark' | 'light'>('theme', () => {
    try {
      return localStorage.getItem('havre-theme') === 'light' ? 'light' : 'dark'
    } catch {
      return 'dark'
    }
  })

  watchEffect(() => {
    document.documentElement.dataset.theme = theme.value
    try {
      localStorage.setItem('havre-theme', theme.value)
    } catch {
      /* private mode: fine, we just won't remember */
    }
  })

  const toggle = () => {
    theme.value = theme.value === 'dark' ? 'light' : 'dark'
  }

  return { theme, toggle }
}
