use std::collections::HashMap;
use uuid::Uuid;

pub fn seeded_ids(content_type: &str) -> HashMap<Uuid, String> {
    let ids: &[&str] = match content_type {
        "post" => &[
            "731b0395-4888-4822-b516-05b4b7bf2089",
            "9601c044-6130-4ee5-a155-96570e05a02f",
            "933dde0f-4744-4a66-9a38-bf5cb1f67553",
            "ea0f2020-0509-45fd-adb9-24b8843055ee",
            "bd27f926-0a00-41fd-b085-a7491e6d0902",
        ],
        "bonus_hunter" => &[
            "c3d4e5f6-a7b8-4012-8def-123456789012",
            "c3d4e5f6-a7b8-4012-8def-123456789013",
            "c3d4e5f6-a7b8-4012-8def-123456789014",
            "c3d4e5f6-a7b8-4012-8def-123456789015",
            "c3d4e5f6-a7b8-4012-8def-123456789016",
        ],
        "top_picks" => &[
            "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5d",
            "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5e",
            "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5f",
            "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c60",
            "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c61",
        ],
        _ => &[
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
            "00000000-0000-4000-8000-000000000003",
            "00000000-0000-4000-8000-000000000004",
            "00000000-0000-4000-8000-000000000005",
        ],
    };

    ids.iter()
        .enumerate()
        .filter_map(|(i, s)| {
            Uuid::parse_str(s)
                .ok()
                .map(|u| (u, format!("{content_type} title {i}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_has_at_least_five_items() {
        for ct in ["post", "bonus_hunter", "top_picks"] {
            let items = seeded_ids(ct);
            assert!(items.len() >= 5, "expected >= 5 items for {ct}");
        }
    }
}
