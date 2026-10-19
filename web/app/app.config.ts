// Nuxt UI's defaults for Havre. Colors themselves come from Havre's tokens
// (assets/css/main.css); these are the palettes Nuxt UI derives shades from.
export default defineAppConfig({
  ui: {
    colors: {
      primary: 'amber',
      neutral: 'stone',
    },
    // A few components tuned to Havre's look (the rest is the theme).
    // Buttons and fields: 36 px high (30 px small), 18 px icons, like before.
    button: {
      variants: {
        size: {
          sm: { base: 'px-2.75 py-1.5 text-[13px] gap-1.5', leadingIcon: 'size-4', trailingIcon: 'size-4' },
          md: { base: 'px-3.5 py-2 text-sm gap-2', leadingIcon: 'size-4.5', trailingIcon: 'size-4.5' },
        },
      },
      compoundVariants: [
        { size: 'sm', square: true, class: 'p-1.75' },
        { size: 'md', square: true, class: 'p-2.25' },
        // Neutral outline = Havre's plain button; neutral ghost = its icon button.
        {
          color: 'neutral',
          variant: 'outline',
          class: 'ring-accented text-default hover:bg-(--hover) active:bg-(--hover)',
        },
        { color: 'neutral', variant: 'ghost', class: 'text-muted hover:text-default hover:bg-(--hover) active:bg-(--hover)' },
        { color: 'primary', variant: 'solid', class: 'hover:bg-primary hover:brightness-108 active:bg-primary' },
      ],
    },
    input: {
      // md: fields (36 px); lg: the search boxes at the top of a list (40 px);
      // xl: Tasks' quick add (46 px).
      variants: { size: { md: { base: 'px-2.5 py-2 text-sm gap-1.5' }, lg: { base: 'px-3 py-2.5 text-sm gap-2' }, xl: { base: 'px-3.5 py-3 text-[14.5px] gap-2.5 rounded-lg' } } },
    },
    select: {
      variants: { size: { md: { base: 'px-2.5 py-2 text-sm gap-1.5' } } },
    },
    selectMenu: {
      variants: { size: { md: { base: 'px-2.5 py-2 text-sm gap-1.5' } } },
    },
    textarea: {
      variants: { size: { md: { base: 'px-2.5 py-2 text-sm gap-1.5' } } },
    },
    popover: {
      slots: { content: 'bg-(--raise) ring-(--line2) rounded-(--radius) shadow-[0_16px_48px_rgb(0_0_0/0.35)]' },
    },
    dropdownMenu: {
      slots: { content: 'bg-(--raise) ring-(--line2) rounded-(--radius) shadow-[0_16px_48px_rgb(0_0_0/0.35)] min-w-56' },
    },
    modal: {
      slots: {
        overlay: 'bg-black/35',
        content: 'bg-(--panel) ring-(--line2) rounded-(--radius-lg) shadow-[0_30px_80px_-20px_rgb(0_0_0/0.55)]',
      },
    },
    slideover: {
      slots: { overlay: 'bg-black/35', content: 'bg-(--panel) ring-(--line2)' },
    },
    // Segmented switches (Day / Week / Month, All / Unread…): Havre's pill.
    tabs: {
      variants: { variant: { pill: { list: 'p-0.5 bg-(--bg) ring ring-default rounded-(--radius-sm)', trigger: 'rounded-md' } } },
      compoundVariants: [
        {
          color: 'neutral',
          variant: 'pill',
          class: { indicator: 'bg-accented', trigger: 'data-[state=active]:text-highlighted' },
        },
      ],
    },
    calendar: {
      variants: { color: { primary: { headCell: 'text-dimmed font-normal' } } },
      slots: { cellTrigger: 'data-today:not-data-[selected]:text-primary' },
    },
    // The Settings menu: quiet rows, the open page on Havre's selection grey.
    // Group titles in the Ctrl+K palette: Havre's small mono capitals.
    commandPalette: {
      slots: { label: 'font-mono font-normal text-[11px] uppercase tracking-[0.06em] text-dimmed' },
    },
    // Key hints as Havre writes them: "Ctrl K", quiet, in the mono font.
    kbd: {
      base: 'normal-case font-mono font-normal rounded-[5px] px-1.5',
      compoundVariants: [{ color: 'neutral', variant: 'outline', class: 'ring-(--line2) text-dimmed' }],
      defaultVariants: { color: 'neutral', variant: 'outline' },
    },
    navigationMenu: {
      slots: { link: 'font-normal text-sm py-2', linkLeadingIcon: 'size-4' },
      compoundVariants: [
        { color: 'neutral', variant: 'pill', active: true, class: { link: 'text-default before:bg-accented' } },
        { color: 'neutral', variant: 'pill', active: false, class: { link: 'text-muted hover:before:bg-(--hover)' } },
      ],
    },
    // Labels as Havre writes them: small and quiet, the field takes the width.
    formField: {
      slots: { label: 'text-muted font-normal text-[12.5px]', help: 'text-dimmed text-xs', container: 'w-full' },
      variants: { orientation: { horizontal: { root: 'place-items-center' } } },
    },
    // Nuxt UI's own icons (close, chevrons, check…) drawn from Havre's set.
    icons: {
      arrowDown: 'i-havre-arrow-down',
      arrowLeft: 'i-havre-arrow-left',
      arrowRight: 'i-havre-arrow-right',
      arrowUp: 'i-havre-arrow-up',
      caution: 'i-havre-alert',
      check: 'i-havre-check',
      chevronDoubleLeft: 'i-havre-back',
      chevronDoubleRight: 'i-havre-next',
      chevronDown: 'i-havre-chevron',
      chevronLeft: 'i-havre-back',
      chevronRight: 'i-havre-next',
      chevronUp: 'i-havre-up',
      close: 'i-havre-close',
      copy: 'i-havre-copy',
      copyCheck: 'i-havre-check',
      dark: 'i-havre-moon',
      drag: 'i-havre-grip',
      ellipsis: 'i-havre-ellipsis',
      error: 'i-havre-error',
      external: 'i-havre-external',
      eye: 'i-havre-eye',
      eyeOff: 'i-havre-eye-off',
      file: 'i-havre-file',
      folder: 'i-havre-folder',
      folderOpen: 'i-havre-folder',
      hash: 'i-havre-hash',
      info: 'i-havre-info',
      light: 'i-havre-sun',
      loading: 'i-havre-loading',
      menu: 'i-havre-menu',
      minus: 'i-havre-minus',
      panelClose: 'i-havre-back',
      panelOpen: 'i-havre-next',
      plus: 'i-havre-plus',
      reload: 'i-havre-refresh',
      search: 'i-havre-search',
      stop: 'i-havre-square',
      star: 'i-havre-star',
      success: 'i-havre-success',
      system: 'i-havre-monitor',
      tip: 'i-havre-info',
      upload: 'i-havre-upload',
      warning: 'i-havre-alert',
    },
  },
})
