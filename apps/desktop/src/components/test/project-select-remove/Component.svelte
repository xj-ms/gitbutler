<script lang="ts">
	import ProjectSelectItem from "$components/shared/ProjectSelectItem.svelte";
	import { Select } from "@gitbutler/ui";

	interface Props {
		onRemove: () => void;
		onSelect: (value: string) => void;
	}

	const { onRemove, onSelect }: Props = $props();

	const projects: Array<{ value: string; label: string }> = [
		{ value: "other-project", label: "Other project" },
	];
	let projectSelector = $state<Select>();
</script>

<Select bind:this={projectSelector} options={projects} onselect={onSelect}>
	{#snippet customSelectButton()}
		<button type="button">Open projects</button>
	{/snippet}

	{#snippet itemSnippet({ item, highlighted })}
		{#if item.value && item.label}
			<ProjectSelectItem
				projectId={item.value}
				projectTitle={item.label}
				{highlighted}
				removable
				onRemove={() => {
					projectSelector?.toggleList();
					onRemove();
				}}
			/>
		{/if}
	{/snippet}
</Select>
