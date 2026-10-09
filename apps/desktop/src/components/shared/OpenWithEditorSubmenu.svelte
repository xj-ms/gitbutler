<script lang="ts">
	import { getEditorOptions } from "$lib/backend/editor";
	import { ContextMenuItem, ContextMenuItemSubmenu, ContextMenuSection } from "@gitbutler/ui";

	type Props = {
		platformName: string;
		defaultEditorId: string;
		disabled?: boolean;
		onselect: (editorId: string) => void | Promise<void>;
	};

	const { platformName, defaultEditorId, disabled = false, onselect }: Props = $props();
	const editors = $derived(
		getEditorOptions(platformName).filter((editor) => editor.schemeIdentifer !== defaultEditorId),
	);
</script>

{#if editors.length > 0}
	<ContextMenuItemSubmenu label="Open with" icon="open-in-ide" {disabled}>
		{#snippet submenu({ close })}
			<ContextMenuSection>
				{#each editors as editor (editor.schemeIdentifer)}
					<ContextMenuItem
						label={editor.displayName}
						onclick={() => {
							close();
							return onselect(editor.schemeIdentifer);
						}}
					/>
				{/each}
			</ContextMenuSection>
		{/snippet}
	</ContextMenuItemSubmenu>
{/if}
