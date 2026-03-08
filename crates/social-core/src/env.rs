/// Loads `.env` if present (no-op in Docker/K8s where env is injected).
pub fn load_dotenv() {
    let _ = dotenvy::dotenv();
}
