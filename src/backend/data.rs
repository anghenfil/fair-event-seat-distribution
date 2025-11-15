use std::collections::HashMap;
use std::cmp::*;
use uuid::Uuid;

use argon2::{Argon2, password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::OsRng;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct Storage{
    pub events: HashMap<Uuid, Event>,
    pub invitations_codes: HashMap<String, Invitation>,
    pub admins: HashMap<String, AdminAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAccount{
    pub username: String,
    /// PHC-format Argon2 hash string
    pub password_hash: String,
}

impl AdminAccount {
    pub fn new_hashed(username: String, password_plain: &str) -> Self {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password_plain.as_bytes(), &salt)
            .expect("argon2 hashing failed");
        AdminAccount { username, password_hash: hash.to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invitation{
    /// Code used to authenticate
    pub code: String,
    /// Reference to event
    pub event_id: Uuid,
    /// Reference to an event's participant entry once the user registered for the event
    pub participant_id: Option<Uuid>,
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage {
    pub fn new() -> Self {
        Storage { events: HashMap::new(), invitations_codes: Default::default(), admins: HashMap::new() }
    }

    pub fn add_admin(&mut self, username: impl Into<String>, password_plain: &str) -> Result<(), &'static str> {
        let username = username.into();
        let acc = AdminAccount::new_hashed(username.clone(), password_plain);
        self.admins.insert(username, acc);
        Ok(())
    }

    pub fn verify_admin(&self, username: &str, password_plain: &str) -> bool {
        match self.admins.get(username) {
            None => false,
            Some(acc) => {
                let Ok(parsed) = PasswordHash::new(&acc.password_hash) else { return false; };
                Argon2::default()
                    .verify_password(password_plain.as_bytes(), &parsed)
                    .is_ok()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event{
    pub uuid: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub slots: Vec<Slot>,
    pub participants: HashMap<uuid::Uuid, Participant>,
    pub state: EventState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum EventState{
    /// The event doesn't allow registrations yet
    #[default]
    NotOpenedYet,
    /// Users can set their preferences
    OpenForRegistration,
    /// The registration is closed, the system is assigning the seats
    AssigningSeats,
    /// The assignment is finished, users can retrieve the result
    Finished
}

impl Event{
    pub fn new(name: String, description: Option<String>) -> Event{
        Event{
            uuid: Uuid::new_v4(),
            name,
            description,
            slots: vec![],
            participants: HashMap::new(),
            state: Default::default(),
        }
    }
    /// Allocates all participants in all slots
    pub fn allocate_participants(&mut self){
        for i in 0..self.slots.len(){
            self.allocate_participants_in_slot(i)
        }
    }

    pub fn allocate_participants_in_slot(&mut self, index: usize) {
        let slot = self.slots.get_mut(index).unwrap();
        while let Some(session_id) = slot.find_session_with_highest_ranked_application() {
            let session = slot.sessions.iter_mut().find(|s| s.uuid == session_id).unwrap(); // We can safely unwrap here

            if session.participants.len() >= session.seats { // Check if all seats in session are taken
                println!("No more seats for session {}!", session.name);
                session.applications = Vec::new(); // Clear applications for session
                continue;
            }

            // Add participant to session participants
            let application = session.applications.remove(0);
            let participant_id = application.participant;

            session.participants.push(participant_id);
            println!("Added participant {} with {:?} points and priority {:?} to session {}.", participant_id, application.calculated_points, application.priority, session.name);

            // Remove participant from all other session applications
            for session in slot.sessions.iter_mut() {
                session.applications.retain_mut(|a| a.participant != participant_id);
            }

            // set persons points from previous round
            match application.priority {
                ApplicationPriority::FirstPreference => {
                    // participant got first preference -> no points
                    if let Some(participant) = self.participants.get_mut(&participant_id) {
                        participant.points_from_previous_rounds = 0;
                    }
                }
                ApplicationPriority::SecondPreference => {
                    // participant got second preference -> add 5 points
                    if let Some(participant) = self.participants.get_mut(&participant_id) {
                        participant.points_from_previous_rounds = 5;
                    }
                }
                ApplicationPriority::ThirdPreference => {
                    // participant got third preference -> add 10 points
                    if let Some(participant) = self.participants.get_mut(&participant_id) {
                        participant.points_from_previous_rounds = 10;
                    }
                },
                ApplicationPriority::NoPreference => {
                    // participant didn't want this session -> add 15 points
                    if let Some(participant) = self.participants.get_mut(&participant_id) {
                        participant.points_from_previous_rounds = 15;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot{
    pub uuid: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub sessions: Vec<Session>,
}
impl Slot{
    pub fn new(name: String, description: Option<String>) -> Self{
        Slot{
            uuid: uuid::Uuid::new_v4(),
            name,
            description,
            sessions: vec![],
        }
    }


    /// Returns the session with the application with the highest calculated_points score across all sessions
    pub fn find_session_with_highest_ranked_application(&self) -> Option<Uuid>{
        let mut highscore = 0;
        let mut highscore_session_id: Option<Uuid> = None;

        for session in &self.sessions {
            if let Some(highest_application) = session.applications.first(){
                if highscore <= highest_application.calculated_points.unwrap_or(0){
                    highscore = highest_application.calculated_points.unwrap_or(0);
                    highscore_session_id = Some(highest_application.session_uuid);
                }
            }
        }

        highscore_session_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session{
    pub uuid: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub seats: usize,
    pub participants: Vec<uuid::Uuid>,
    pub applications: Vec<Application>,
}

impl Session{
    pub fn new(name: String, description: Option<String>, seats: usize) -> Session{
        Session{
            uuid: uuid::Uuid::new_v4(),
            name,
            description,
            seats,
            participants: vec![],
            applications: vec![],
        }
    }
    pub fn rank_applications(&mut self, event: &Event){
        // remove invalid applications and calculate points for each application
        self.applications.retain_mut(|application|{
            match event.participants.get(&application.participant) {
                None => {
                    eprintln!("Participant id {} from application not found in event {}. Removing application. ", application.participant, event.name);
                    false
                }
                Some(participant) => {
                    application.calculate_points(participant);
                    true
                }
            }
        });
        // Sort descending by points, via uuid if equal points
        self.applications.sort_by(|a, b|b.cmp(a));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApplicationPriority{
    FirstPreference,
    SecondPreference,
    ThirdPreference,
    NoPreference
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application{
    pub uuid: uuid::Uuid,
    pub session_uuid: uuid::Uuid,
    pub participant: uuid::Uuid,
    pub priority: ApplicationPriority,
    pub calculated_points: Option<usize>,
}

impl Ord for Application{
    fn cmp(&self, other: &Self) -> Ordering {
        self.calculated_points.cmp(&other.calculated_points).then(
            self.uuid.cmp(&other.uuid) // We use the uuid to induce randomness for applications with same number of points
        )
    }
}

impl PartialOrd for Application{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Application{
    fn eq(&self, other: &Self) -> bool {
        self.calculated_points.unwrap_or(0) == other.calculated_points.unwrap_or(0) && self.uuid == other.uuid
    }
}

impl Eq for Application{

}

impl Application {
    pub fn calculate_points(&mut self, participant: &Participant){
        let mut points = 0;
        if participant.points_from_previous_rounds != 0{
            points += participant.points_from_previous_rounds;
        }
        points += match self.priority{
            ApplicationPriority::FirstPreference => {
                15
            }
            ApplicationPriority::SecondPreference => {
                10
            }
            ApplicationPriority::ThirdPreference => {
                5
            },
            ApplicationPriority::NoPreference => {
                0
            }
        };
        self.calculated_points = Some(points);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub uuid: uuid::Uuid,
    pub name: String,
    pub points_from_previous_rounds: usize,
}

