use dotenv::dotenv;

mod player;

use player::Player;

fn main() {
    dotenv().ok();

    let username = std::env::var("LUCCA_EMAIL").unwrap();
    let password = std::env::var("LUCCA_PASSWORD").unwrap();
    let lucca_url = std::env::var("LUCCA_URL").unwrap();

    let learning = std::env::var("LUCCA_LEARNING").ok().is_some();
    if learning {
        println!("Starting in learning mode");
    }

    let mut player = Player::new(&lucca_url, learning).unwrap();
    player.login(&username, &password).unwrap();

    let game = player.start_game().unwrap();
    let mut scores = vec![];
    for i in 0..game.nb_questions {
        let score = player.guess(&game).unwrap();
        scores.push(score);
        println!("Scored {} at question {}", score, i + 1);
    }

    println!("Total score: {}", scores.iter().sum::<i32>());

    player.save_hash_map().unwrap();
}
