<template>
    <template v-for="(node, i) in nodes" :key="i">
        <br v-if="node.type === 'br'" />
        <a
            v-else-if="node.type === 'link'"
            :href="node.href"
            target="_blank"
            rel="noopener noreferrer"
        >{{ node.text }}</a>
        <a
            v-else-if="node.type === 'anchor'"
            :href="`#${node.target}`"
            @click.prevent="$emit('anchor', node.target)"
        >{{ node.text }}</a>
        <strong v-else-if="node.type === 'strong'">{{ node.text }}</strong>
        <em v-else-if="node.type === 'em'">{{ node.text }}</em>
        <code v-else-if="node.type === 'code'">{{ node.text }}</code>
        <template v-else>{{ node.text }}</template>
    </template>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { MarkdownInline } from '@/utils/safeText'

    /**
     * The inline content of a parsed Markdown block (utils/safeText.ts
     * `parseSafeMarkdown`): text, emphasis, code and links, each through a
     * Vue text interpolation, so nothing is ever parsed as HTML. External
     * links open in the browser; a link to a heading of the same document
     * emits `anchor` with the heading's id instead of changing the address.
     */
    export default defineComponent({
        name: 'MarkdownInlines',

        props: {
            nodes: { type: Array as PropType<MarkdownInline[]>, required: true },
        },

        emits: ['anchor'],
    })
</script>
