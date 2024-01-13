use std::io::Write;
use std::{
    collections::HashMap,
    fmt::Debug,
    fs::{read_to_string, File},
    path::Path,
};

use ahash::RandomState;
use anyhow::{anyhow, Result};

use scraper::{Html, Selector};

use serde::Deserialize;
use url::Url;

pub struct PlayerOptions {
    lucca_url: Url,
    training: bool,
}

pub struct Player {
    client: reqwest::blocking::Client,
    options: PlayerOptions,
    hash_map: HashMap<u64, String>,
}

#[derive(Deserialize, Debug)]
pub struct Game {
    id: String,
    #[serde(rename = "nbQuestions")]
    pub nb_questions: u32,
}

#[derive(Deserialize, Debug)]
struct Question {
    id: u32,
    #[serde(rename = "imageUrl")]
    image_url: String,
    suggestions: [Suggestion; 4],
}

#[derive(Deserialize, Debug)]
struct Suggestion {
    id: u32,
    value: String,
}

#[derive(Deserialize, Debug)]
struct GuessResponse {
    score: i32,
    #[serde(rename = "isCorrect")]
    is_correct: bool,
    #[serde(rename = "correctSuggestionId")]
    correct_suggestion_id: u32,
}

const LOGIN_ADDR: &str = "identity/login";
const FACES_ADDR: &str = "faces/api";

static HASHER: RandomState = RandomState::with_seeds(
    10960905448801897020,
    6565933669389301275,
    5017652980937232669,
    4134542598451985848,
);

impl Player {
    pub fn new(lucca_url: &str, training: bool) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .build()?;

        let options = PlayerOptions {
            lucca_url: Url::parse(lucca_url)?,
            training,
        };

        let hash_file_path = "data";
        let path = Path::new(hash_file_path);
        let hash_map = match path.exists() {
            true => ron::from_str(read_to_string(path)?.as_str())?,
            false => HashMap::<u64, String>::new(),
        };

        Ok(Self {
            client,
            options,
            hash_map,
        })
    }

    pub fn reload_hash_map(&mut self) -> Result<()> {
        let hash_file_path = "data";
        let path = Path::new(hash_file_path);
        self.hash_map = match path.exists() {
            true => ron::from_str(read_to_string(path)?.as_str())?,
            false => HashMap::<u64, String>::new(),
        };

        Ok(())
    }

    pub fn save_hash_map(&self) -> Result<()> {
        let hash_file_path = "data";
        let path = Path::new(hash_file_path);

        let mut file = File::create(path)?;

        file.write_all(&ron::to_string(&self.hash_map)?.into_bytes())?;
        Ok(())
    }

    pub fn login(&self, username: &str, password: &str) -> Result<()> {
        let login_url = self.options.lucca_url.join(&LOGIN_ADDR)?;
        let response = self.client.get(login_url.clone()).send()?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "GET request to {} resulted in a code {}",
                &login_url,
                response.status()
            ));
        }

        let html = Html::parse_document(response.text()?.as_str());
        let selector = Selector::parse("input[name=\"__RequestVerificationToken\"]").unwrap();
        let verification_token = html
            .select(&selector)
            .next()
            .ok_or(anyhow!("Failed to retrieve the verification token element"))?
            .value()
            .attr("value")
            .ok_or(anyhow!("Failed to retrieve the verification token"))?;

        let mut login_form = HashMap::new();
        login_form.insert("ReturnUrl", "/home");
        login_form.insert("UserName", username);
        login_form.insert("Password", password);
        login_form.insert("IsPersistent", "true");
        login_form.insert("__RequestVerificationToken", verification_token);
        let response = self.client.post(login_url).form(&login_form).send()?;

        if response.status().is_success() {
            return Ok(());
        }

        Err(anyhow!("Failed to log in"))
    }

    pub fn start_game(&self) -> Result<Game> {
        let mut url_str = FACES_ADDR.to_owned() + "/games";
        if self.options.training {
            url_str += "/training";
        }

        let mut training_form = HashMap::new();
        training_form.insert("departmentIds", Vec::<usize>::new());
        training_form.insert("establishmentIds", vec![]);

        let game_url = self.options.lucca_url.join(&url_str)?;
        let request = match self.options.training {
            true => self.client.post(game_url).json(&training_form),
            false => self
                .client
                .post(game_url)
                .json(&HashMap::<String, String>::new()),
        };
        let response = request.send()?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "POST request to {} resulted in a code {}",
                url_str,
                response.status()
            ));
        }

        let game = response.json()?;

        Ok(game)
    }

    pub fn guess(&mut self, game: &Game) -> Result<i32> {
        let url_str = FACES_ADDR.to_owned() + "/games/" + game.id.as_str() + "/questions/next";
        let next_url = self.options.lucca_url.join(&url_str)?;
        let response = self
            .client
            .post(next_url)
            .json(&HashMap::<String, String>::new())
            .send()?;

        let question: Question = response.json()?;

        let url_str = self.options.lucca_url.join(&question.image_url)?;
        let image = self
            .client
            .get(url_str)
            .header("Range", "bytes=0-1023")
            .send()?
            .bytes()?;

        let image_hash = HASHER.hash_one(image);
        let suggestion = match self.hash_map.get(&image_hash) {
            Some(name) => question
                .suggestions
                .iter()
                .filter(|s| &s.value == name)
                .next()
                .unwrap(),
            None => question.suggestions.first().unwrap(),
        };

        let response = self.respond(game, &question, &suggestion)?;
        let correct_suggestion = match response.is_correct {
            true => suggestion,
            false => question
                .suggestions
                .iter()
                .filter(|s| s.id == response.correct_suggestion_id)
                .next()
                .unwrap(),
        };
        // self.reload_hash_map()?;
        self.hash_map
            .insert(image_hash, correct_suggestion.value.clone());

        // self.save_hash_map()?;
        Ok(response.score)
    }

    fn respond(
        &self,
        game: &Game,
        question: &Question,
        suggestion: &Suggestion,
    ) -> Result<GuessResponse> {
        let url_str = FACES_ADDR.to_owned()
            + "/games/"
            + game.id.as_str()
            + "/questions/"
            + question.id.to_string().as_str()
            + "/guess";
        let guess_url = self.options.lucca_url.join(&url_str)?;
        let mut guess_form = HashMap::new();
        guess_form.insert("questionId", question.id);
        guess_form.insert("suggestionId", suggestion.id);
        let response = self.client.post(guess_url).json(&guess_form).send()?;
        let guess_response = response.json()?;

        Ok(guess_response)
    }
}
