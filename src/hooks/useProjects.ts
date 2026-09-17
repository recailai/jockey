import { createSignal } from "solid-js";
import { projectApi, type Project } from "../lib/tauriApi";

export function useProjects(showToast: (message: string, severity?: "error" | "info") => void) {
  const [projects, setProjects] = createSignal<Project[]>([]);
  const [currentProject, setCurrentProject] = createSignal<Project | null>(null);
  const [loading, setLoading] = createSignal(false);

  const refreshProjects = async () => {
    try {
      setLoading(true);
      const list = await projectApi.list();
      setProjects(list);
      if (list.length > 0) {
        const savedId = localStorage.getItem("jockey_active_project_id");
        const found = list.find((p) => p.id === savedId) ?? list[0];
        setCurrentProject(found);
      } else {
        setCurrentProject(null);
      }
    } catch (e) {
      showToast(`Failed to load projects: ${String(e)}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const selectProject = (project: Project) => {
    setCurrentProject(project);
    try {
      localStorage.setItem("jockey_active_project_id", project.id);
    } catch { /* ignore */ }
  };

  const createProject = async (name: string, rootPath: string) => {
    try {
      const proj = await projectApi.create(name, rootPath);
      await refreshProjects();
      selectProject(proj);
      showToast(`Project '${proj.name}' added`, "info");
      return proj;
    } catch (e) {
      showToast(`Failed to create project: ${String(e)}`, "error");
      throw e;
    }
  };

  const deleteProject = async (id: string) => {
    try {
      await projectApi.remove(id);
      await refreshProjects();
      showToast("Project deleted", "info");
    } catch (e) {
      showToast(`Failed to delete project: ${String(e)}`, "error");
    }
  };

  return {
    projects,
    currentProject,
    selectProject,
    createProject,
    deleteProject,
    refreshProjects,
    loading,
  };
}
