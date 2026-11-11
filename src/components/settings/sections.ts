/**
 * The Settings screen's sections, in the order of its side navigation.
 * Each has a deep link, `/settings#<id>` (the native menu's Privacy item
 * opens `#privacy`). Some settings have their own anchor inside a section
 * (`#mtp-setting`, linked from the sidebar's MTP notice): `sectionFromHash`
 * opens the section that holds it and names the element to scroll to.
 */
export const SETTINGS_SECTIONS = [
    { id: 'library', icon: 'book' },
    { id: 'devices', icon: 'tablet' },
    { id: 'metadata', icon: 'search' },
    { id: 'opds', icon: 'radio' },
    { id: 'catalogs', icon: 'globe' },
    { id: 'appearance', icon: 'sun' },
    { id: 'privacy', icon: 'shield' },
    { id: 'updates', icon: 'download-cloud' },
    { id: 'backup', icon: 'archive' },
    { id: 'advanced', icon: 'tool' },
] as const

export type SettingsSectionId = typeof SETTINGS_SECTIONS[number]['id']

/** Anchors of single settings, and the section each one is in. */
export const SETTING_ANCHORS: Record<string, SettingsSectionId> = {
    'mtp-setting': 'devices',
    'opds-port': 'opds',
    'google-books-key': 'metadata',
}

export const DEFAULT_SECTION: SettingsSectionId = 'library'

export function isSettingsSection(id: string): id is SettingsSectionId {
    return SETTINGS_SECTIONS.some(s => s.id === id)
}

/**
 * What an address's hash opens: `#devices` → the Devices section;
 * `#mtp-setting` → the Devices section, scrolled to the MTP setting;
 * nothing or an unknown hash → the first section.
 */
export function sectionFromHash(hash: string): { section: SettingsSectionId, anchor: string | null } {
    const id = decodeURIComponent(hash.replace(/^#/, ''))
    if (isSettingsSection(id)) { return { section: id, anchor: null } }
    const section = SETTING_ANCHORS[id] as SettingsSectionId | undefined
    if (section) { return { section, anchor: id } }
    return { section: DEFAULT_SECTION, anchor: null }
}

/** One choice of a ChoiceGroup (segmented radio buttons). */
export interface ChoiceOption {
    value: string
    label: string
    /** Feather icon before the label. */
    icon?: string
}
