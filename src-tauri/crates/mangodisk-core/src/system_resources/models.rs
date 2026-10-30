use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOverview {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub swap_used_bytes: u64,
    pub used_percent: u8,
    pub pressure: mangodisk_platform::system_resources::memory::MemoryPressure,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationIdentity {
    pub id: String,
    pub name: String,
    pub process_count: u32,
    /// Bundle or executable location for native icons and file-manager navigation; never telemetry.
    pub icon_path: Option<String>,
    pub is_bundle: bool,
    pub can_quit: bool,
    pub can_force_quit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationMemory {
    #[serde(flatten)]
    pub application: ApplicationIdentity,
    pub used_bytes: Option<u64>,
    /// Number of readable members included in the sum; zero keeps the value unavailable.
    pub readable_process_count: u32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProcess {
    pub pid: u32,
    pub started_at: u64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCpu {
    pub processes: Vec<CpuProcess>,
    pub location_status: mangodisk_platform::system_resources::process_cpu::ProcessLocationStatus,
    pub pid: u32,
    #[serde(flatten)]
    pub application: ApplicationIdentity,
    /// Percentage in the summary's native display scale; macOS may exceed 100%.
    pub used_percent: f64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCpuSummary {
    pub usage_scale: mangodisk_platform::system_resources::process_cpu::CpuUsageScale,
    pub applications: Vec<ApplicationCpu>,
    pub readable_process_count: u32,
    pub omitted_process_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMemorySummary {
    pub usage_kind: mangodisk_platform::system_resources::memory::ProcessMemoryKind,
    pub applications: Vec<ApplicationMemory>,
    pub readable_process_count: u32,
    /// Unknown native counters, excluded from numeric sums even when their rows remain visible.
    pub omitted_process_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemResourceSnapshot {
    pub schema_version: u32,
    pub sampled_at_ms: u64,
    pub memory: MemoryOverview,
    pub processes: Option<ProcessMemorySummary>,
}
