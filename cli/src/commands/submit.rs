use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize)]
struct SubmitResponse {
    loop_id: uuid::Uuid,
    branch: String,
    state: String,
}

pub struct SubmitArgs<'a> {
    pub engineer: &'a str,
    pub spec_path: &'a str,
    pub harden: bool,
    pub harden_only: bool,
    pub auto_approve: bool,
    pub model_impl: Option<String>,
    pub model_review: Option<String>,
}

pub async fn run(client: &NemoClient, args: SubmitArgs<'_>) -> Result<()> {
    let mut body = serde_json::json!({
        "spec_path": args.spec_path,
        "engineer": args.engineer,
        "harden": args.harden,
        "harden_only": args.harden_only,
        "auto_approve": args.auto_approve,
    });

    if args.model_impl.is_some() || args.model_review.is_some() {
        body["model_overrides"] = serde_json::json!({
            "implementor": args.model_impl,
            "reviewer": args.model_review,
        });
    }

    let resp: SubmitResponse = client.post("/submit", &body).await?;

    println!("Submitted loop {}", resp.loop_id);
    println!("  Branch: {}", resp.branch);
    println!("  State:  {}", resp.state);

    if !args.auto_approve && !args.harden_only {
        println!(
            "\n  Run `nemo approve {}` to start implementation.",
            resp.loop_id
        );
    }

    Ok(())
}
