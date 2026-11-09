<script lang="ts">
	import { goto } from "$app/navigation";
	import ProjectSetupTarget from "$components/onboarding/ProjectSetupTarget.svelte";
	import IllustrationSplitLayout from "$components/shared/IllustrationSplitLayout.svelte";
	import ReduxResult from "$components/shared/ReduxResult.svelte";
	import newZenSvg from "$lib/assets/illustrations/new-zen.svg?raw";
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { PROJECTS_SERVICE } from "$lib/project/projectsService";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
	import { OnboardingEvent, POSTHOG_WRAPPER } from "$lib/telemetry/posthog";
	import { inject } from "@gitbutler/core/context";
	import { TestId } from "@gitbutler/ui";
	import type { RemoteBranchInfo } from "$lib/baseBranch/baseBranch";

	interface Props {
		projectId: string;
		remoteBranches: RemoteBranchInfo[];
		remoteTags?: RemoteBranchInfo[];
	}

	const { projectId, remoteBranches, remoteTags = [] }: Props = $props();

	const TAG_PREFIX = "refs/tags/";

	const projectsService = inject(PROJECTS_SERVICE);
	const baseService = inject(BASE_BRANCH_SERVICE);
	const posthog = inject(POSTHOG_WRAPPER);
	const settingsStore = inject(SETTINGS_SERVICE).appSettings;
	const projectQuery = $derived(projectsService.getProject(projectId));
	const [setBaseBranchTarget, targetBranchSwitch] = baseService.setTarget;
	const [setBaseBranchTargetRef, targetRefSwitch] = baseService.setTargetRef;
	const settingTarget = $derived(
		targetBranchSwitch.current.isLoading || targetRefSwitch.current.isLoading,
	);

	async function setTarget(branch: string[]) {
		const target = branch[0];
		if (!target) return;
		const isTag = target.startsWith(TAG_PREFIX);
		const pushRemote = isTag ? undefined : branch[1] || undefined;

		try {
			if ($settingsStore?.featureFlags.singleBranch) {
				// Only set the target; the user keeps working on their current branch.
				const targetRef = isTag ? target : `refs/remotes/${target}`;
				await setBaseBranchTargetRef({
					projectId: projectId,
					targetRef,
					pushRemote,
				});
			} else {
				await setBaseBranchTarget({
					projectId: projectId,
					branch: target,
					pushRemote,
				});
			}
			posthog.captureOnboarding(OnboardingEvent.SetTargetBranch);
			goto(`/${projectId}/`, { invalidateAll: true });
		} catch (e: unknown) {
			posthog.captureOnboarding(OnboardingEvent.SetTargetBranchFailed, e);
		}
	}

	$effect(() => {
		if (projectQuery.result.isError) {
			console.error("Failed to load project, redirecting:", projectQuery.result.error);
			goto("/");
		}
	});
</script>

<IllustrationSplitLayout img={newZenSvg} testId={TestId.ProjectSetupPage}>
	<ReduxResult {projectId} result={projectQuery.result}>
		{#snippet children(project)}
			<ProjectSetupTarget
				{projectId}
				projectName={project.title}
				{remoteBranches}
				{remoteTags}
				loading={settingTarget}
				onBranchSelected={async (branch) => {
					await setTarget(branch);
				}}
			/>
		{/snippet}
	</ReduxResult>
</IllustrationSplitLayout>
