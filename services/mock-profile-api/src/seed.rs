use std::collections::HashMap;

pub fn tokens() -> HashMap<String, (String, String)> {
    let mut tokens = HashMap::new();
    tokens.insert(
        "tok_user_1".to_string(),
        (
            "usr_550e8400-e29b-41d4-a716-446655440001".to_string(),
            "Test User 1".to_string(),
        ),
    );
    tokens.insert(
        "tok_user_2".to_string(),
        (
            "usr_550e8400-e29b-41d4-a716-446655440002".to_string(),
            "Test User 2".to_string(),
        ),
    );
    tokens.insert(
        "tok_user_3".to_string(),
        (
            "usr_550e8400-e29b-41d4-a716-446655440003".to_string(),
            "Test User 3".to_string(),
        ),
    );
    tokens.insert(
        "tok_user_4".to_string(),
        (
            "usr_550e8400-e29b-41d4-a716-446655440004".to_string(),
            "Test User 4".to_string(),
        ),
    );
    tokens.insert(
        "tok_user_5".to_string(),
        (
            "usr_550e8400-e29b-41d4-a716-446655440005".to_string(),
            "Test User 5".to_string(),
        ),
    );
    tokens
}
