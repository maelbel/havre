// Dark by default, light on demand. Nuxt's color mode keeps the choice (in
// this browser, under `havre-theme`) and sets `<html class="dark"
// data-theme="dark">`: the class for Nuxt UI, the attribute for Havre's CSS.
export function useTheme() {
  const mode = useColorMode()
  const theme = computed<'dark' | 'light'>({
    get: () => (mode.value === 'light' ? 'light' : 'dark'),
    set: (value) => {
      mode.preference = value
    },
  })
  const toggle = () => {
    theme.value = theme.value === 'dark' ? 'light' : 'dark'
  }
  return { theme, toggle }
}
