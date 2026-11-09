<script lang="ts">
	import { goto } from "$app/navigation";
	import RemoveProjectModal from "$components/projectSettings/RemoveProjectModal.svelte";
	import ProjectSelectItem from "$components/shared/ProjectSelectItem.svelte";
	import { handleAddProjectOutcome } from "$lib/project/project";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { projectPath } from "$lib/routes/routes.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, OptionsGroup, Select, SelectItem, chipToasts } from "@gitbutler/ui";
	import { tick, untrack } from "svelte";

	const { projectId }: { projectId?: string } = $props();

	const projectsService = inject(PROJECTS_SERVICE);
	const projectsQuery = $derived(projectsService.projects());
	const serverCapabilitiesQuery = $derived(projectsService.serverCapabilities());
	const canAddProjects = $derived(serverCapabilitiesQuery.response?.canAddProjects ?? true);

	let selectedId = $state<string | undefined>(untrack(() => projectId));

	const mappedProjects = $derived(
		projectsQuery.response?.map((project) => ({
			value: project.id,
			label: project.title,
		})) || [],
	);

	let newProjectLoading = $state(false);
	let cloneProjectLoading = $state(false);
	let projectSelector = $state<Select>();
	let projectToRemove = $state<{ id: string; title: string }>();
	let removeProjectModal = $state<RemoveProjectModal>();
	let removingProject = $state(false);

	async function confirmProjectRemoval(id: string, title: string) {
		projectSelector?.toggleList();
		projectToRemove = { id, title };
		await tick();
		removeProjectModal?.show();
	}

	async function removeProject() {
		if (!projectToRemove) return;

		removingProject = true;
		try {
			if (selectedId === projectToRemove.id) {
				selectedId = projectId;
			}
			await projectsService.deleteProject(projectToRemove.id);
			chipToasts.success("Project deleted");
		} finally {
			removingProject = false;
		}
	}
</script>

<div class="project-switcher">
	<Select
		bind:this={projectSelector}
		value={selectedId}
		options={mappedProjects}
		label="Switch to another project"
		wide
		onselect={(value) => {
			selectedId = value;
		}}
		searchable
	>
		{#snippet itemSnippet({ item, highlighted })}
			{#if item.value && item.label}
				<ProjectSelectItem
					projectId={item.value}
					projectTitle={item.label}
					selected={item.value === selectedId}
					{highlighted}
					removable={item.value !== projectId}
					onRemove={() => confirmProjectRemoval(item.value!, item.label!)}
				/>
			{/if}
		{/snippet}

		<OptionsGroup>
			{#if canAddProjects}
				<SelectItem
					icon="plus"
					loading={newProjectLoading}
					onClick={async () => {
						newProjectLoading = true;
						try {
							const outcome = await projectsService.addProject();
							if (!outcome) {
								// User cancelled the project creation
								newProjectLoading = false;
								return;
							}
							handleAddProjectOutcome(outcome, (project) => goto(projectPath(project.id)));
						} finally {
							newProjectLoading = false;
						}
					}}
				>
					Add local repository
				</SelectItem>
			{/if}
			<SelectItem
				icon="clone"
				loading={cloneProjectLoading}
				onClick={async () => {
					cloneProjectLoading = true;
					try {
						goto("/onboarding/clone");
					} finally {
						cloneProjectLoading = false;
					}
				}}
			>
				Clone repository
			</SelectItem>
		</OptionsGroup>
	</Select>

	<Button
		style="pop"
		icon="chevron-right"
		disabled={selectedId === projectId}
		onclick={() => {
			if (selectedId) goto(projectPath(selectedId));
		}}
	>
		Open project
	</Button>
</div>

<RemoveProjectModal
	bind:this={removeProjectModal}
	projectTitle={projectToRemove?.title}
	isDeleting={removingProject}
	onDeleteClicked={removeProject}
/>

<style lang="postcss">
	.project-switcher {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 10px;
	}
</style>
