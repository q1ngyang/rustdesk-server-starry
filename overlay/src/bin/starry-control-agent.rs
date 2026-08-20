use hbb_common::ResultType;

fn main() -> ResultType<()> {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("STARRY_CONTROL_AGENT_CONFIG").ok())
        .unwrap_or_else(|| "starry-control-agent.yaml".to_owned());
    let runtime = hbb_common::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime
        .block_on(hbbs::control_agent::run(path))
        .map_err(hbb_common::anyhow::Error::msg)
}
