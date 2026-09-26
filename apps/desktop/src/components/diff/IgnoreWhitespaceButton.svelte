<script lang="ts">
	import { UI_STATE } from "$lib/state/uiState.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button } from "@gitbutler/ui";

	const uiState = inject(UI_STATE);
	const ignoreWhitespace = uiState.global.ignoreWhitespace;
</script>

<Button
	kind="ghost"
	icon="text-contain"
	size="tag"
	activated={ignoreWhitespace.current}
	tooltip={ignoreWhitespace.current
		? "Showing changes without whitespace (hunk actions disabled)"
		: "Ignore whitespace changes"}
	onclick={() => ignoreWhitespace.set(!ignoreWhitespace.current)}
/>
