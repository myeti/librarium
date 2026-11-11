<template>
    <div
        class="theme-picker"
        role="radiogroup"
        :aria-labelledby="labelledby || undefined"
        :aria-describedby="describedby || undefined"
    >
        <label
            v-for="theme in themes"
            :key="theme"
            class="edition"
            :class="{ '-checked': theme === modelValue }"
        >
            <input
                type="radio"
                class="edition__input"
                name="theme"
                :value="theme"
                :checked="theme === modelValue"
                @change="$emit('update:modelValue', theme)"
            />
            <!-- A title page in the theme's own colours (themes.css applies to
                 any element with data-theme). -->
            <span class="edition__page" :data-theme="previewOf(theme)" aria-hidden="true">
                <span class="ornament -corner edition__corner -tl"></span>
                <span class="ornament -corner edition__corner -tr"></span>
                <span class="ornament -art edition__art"></span>
                <span class="edition__wordmark display-title foil-text">Librarium</span>
                <span class="ornament -rule edition__rule"></span>
                <span class="edition__shelf">
                    <span v-for="n in 4" :key="n" class="edition__book" :style="{ background: `var(--cover-${n})` }"></span>
                </span>
                <vue-feather v-if="theme === modelValue" type="check" size="14" class="edition__check" />
            </span>
            <span class="edition__name">{{ $t(`settings.theme.${theme}`) }}</span>
            <span class="edition__tagline">{{ $t(`settings.theme.${theme}Tagline`) }}</span>
        </label>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { dataThemeOf, THEME_NAMES, type ThemeName } from '@/stores/theme'

    /**
     * The theme choice of Settings › Appearance: one card per theme, each a
     * small title page drawn in that theme (its sky, lettering, ornaments and
     * four of its cover cloths). A native radio group: arrow keys move
     * between themes.
     */
    export default defineComponent({
        name: 'ThemePicker',

        props: {
            modelValue:  { type: String as PropType<ThemeName>, required: true },
            /** Id of the element naming the group (SettingRow's label). */
            labelledby:  { type: String, default: '' },
            describedby: { type: String, default: '' },
        },

        emits: ['update:modelValue'],

        data() {
            return { themes: THEME_NAMES }
        },

        methods: {
            /** The `data-theme` of a theme's preview ('' is Nocturne, the default). */
            previewOf(theme: ThemeName): string {
                return dataThemeOf(theme)
            },
        },
    })
</script>

<style lang="scss" scoped>
.theme-picker {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-4);
    width: 100%;
}

.edition {
    display: flex;
    flex-direction: column;
    gap: 3px;
    cursor: pointer;
    border-radius: var(--radius);
}

// The radio stays in the page (for keyboards and screen readers), unseen.
.edition__input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
}

.edition__page {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    aspect-ratio: 4 / 3;
    margin-bottom: 8px;
    overflow: hidden;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    color: var(--text-primary);
    background:
        var(--motif),
        radial-gradient(90% 70% at 50% 30%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%),
        var(--sky);
    box-shadow: var(--shadow-sm);
    transition:
        transform var(--duration-slow) var(--ease-lift),
        box-shadow var(--duration-slow) var(--ease-out),
        border-color var(--transition);
}

.edition:hover .edition__page {
    transform: translateY(-3px);
    box-shadow: var(--shadow);
}

.edition.-checked .edition__page {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent), var(--glow-card);
}

// Keyboard focus on the hidden radio shows on its card.
.edition:has(.edition__input:focus-visible) .edition__page {
    outline: var(--focus-ring-width) solid var(--focus-ring-color);
    outline-offset: 3px;
}

.edition__corner {
    position: absolute;
    top: 6px;
    width: 26px;
    opacity: .8;

    &.-tl { left: 6px; }
    &.-tr { right: 6px; transform: scaleX(-1); }
}

.edition__art {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 64px;
    transform: translate(-50%, -58%);
    opacity: .12;
}

.edition__wordmark {
    position: relative;
    font-size: 17px;
    line-height: 1;
    padding-left: var(--display-tracking);
    white-space: nowrap;
}

.edition__rule {
    position: relative;
    width: 70px;
}

.edition__shelf {
    position: absolute;
    bottom: 0;
    left: 50%;
    display: flex;
    gap: 3px;
    transform: translateX(-50%);
}

.edition__book {
    width: 16px;
    height: 22px;
    border-radius: 1px 2px 0 0;
    box-shadow: inset 1px 0 0 rgba(255,255,255,.18), inset 0 0 0 1px rgba(255,255,255,.08), 0 -2px 6px rgba(0,0,0,.25);
}

.edition__check {
    position: absolute;
    top: 8px;
    right: 8px;
    padding: 3px;
    border-radius: 50%;
    background: var(--accent-strong);
    color: var(--on-accent);
    box-sizing: content-box;
}

.edition__name {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.edition__tagline {
    font-size: var(--font-size-xs);
    line-height: var(--line-height-tight);
    color: var(--text-muted);
}

@media (prefers-reduced-motion: reduce) {
    .edition:hover .edition__page { transform: none; }
}
</style>
