use crate::{output_manager::OutputManager, target::Target};
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant},
};

pub struct MidiPlayer {
    playback: midi_file::PlaybackState,
    output_manager: Rc<RefCell<OutputManager>>,
    midi_file: Rc<midi_file::Midi>,
    play_along: PlayAlong,
}

impl MidiPlayer {
    pub fn new(target: &mut Target, user_keyboard_range: piano_math::KeyboardRange) -> Self {
        let midi_file = target.midi_file.as_ref().unwrap();

        let mut player = Self {
            playback: midi_file::PlaybackState::new(
                Duration::from_secs(3),
                &midi_file.merged_track,
            ),
            output_manager: target.output_manager.clone(),
            midi_file: midi_file.clone(),
            play_along: PlayAlong::new(user_keyboard_range),
        };
        player.update(target, Duration::ZERO);

        player
    }

    /// When playing: returns midi events
    ///
    /// When paused: returns None
    pub fn update(
        &mut self,
        target: &mut Target,
        delta: Duration,
    ) -> Option<Vec<midi_file::MidiEvent>> {
        self.play_along.update();

        let elapsed = (delta / 10) * (target.config.speed_multiplier * 10.0) as u32;

        let events = self.playback.update(&self.midi_file.merged_track, elapsed);

        events.iter().for_each(|event| {
            self.output_manager.borrow_mut().midi_event(event);

            if event.channel == 9 {
                return;
            }

            use midi_file::midly::MidiMessage;
            match event.message {
                MidiMessage::NoteOn { key, .. } => {
                    self.play_along
                        .press_key(KeyPressSource::File, key.as_int(), true);
                }
                MidiMessage::NoteOff { key, .. } => {
                    self.play_along
                        .press_key(KeyPressSource::File, key.as_int(), false);
                }
                _ => {}
            }
        });

        if self.playback.is_paused() {
            None
        } else {
            Some(events)
        }
    }

    fn clear(&mut self) {
        self.output_manager.borrow_mut().stop_all();
    }
}

impl Drop for MidiPlayer {
    fn drop(&mut self) {
        self.clear();
    }
}

impl MidiPlayer {
    pub fn start(&mut self) {
        self.resume();
    }

    pub fn pause_resume(&mut self) {
        if self.playback.is_paused() {
            self.resume();
        } else {
            self.pause();
        }
    }

    pub fn pause(&mut self) {
        self.clear();
        self.playback.pause();
    }

    pub fn resume(&mut self) {
        self.playback.resume();
    }

    fn set_time(&mut self, time: Duration) {
        self.playback.set_time(time);

        // Discard all of the events till that point
        let events = self
            .playback
            .update(&self.midi_file.merged_track, Duration::ZERO);
        std::mem::drop(events);

        self.clear();
    }

    pub fn rewind(&mut self, delta: i64) {
        let mut time = self.playback.time();

        if delta < 0 {
            let delta = Duration::from_millis((-delta) as u64);
            time = time.saturating_sub(delta);
        } else {
            let delta = Duration::from_millis(delta as u64);
            time = time.saturating_add(delta);
        }

        self.set_time(time);
    }

    pub fn set_percentage_time(&mut self, p: f32) {
        self.set_time(Duration::from_secs_f32(
            (p * self.playback.lenght().as_secs_f32()).max(0.0),
        ));
    }

    pub fn percentage(&self) -> f32 {
        self.playback.percentage()
    }

    pub fn time_without_lead_in(&self) -> f32 {
        self.playback.time().as_secs_f32() - self.playback.leed_in().as_secs_f32()
    }

    pub fn is_paused(&self) -> bool {
        self.playback.is_paused()
    }
}

impl MidiPlayer {
    pub fn play_along(&self) -> &PlayAlong {
        &self.play_along
    }

    pub fn play_along_mut(&mut self) -> &mut PlayAlong {
        &mut self.play_along
    }
}

pub enum KeyPressSource {
    File,
    User,
}

#[derive(Debug)]
struct UserPress {
    timestamp: Instant,
    note_id: u8,
}

#[derive(Debug)]
pub struct PlayAlong {
    user_keyboard_range: piano_math::KeyboardRange,

    required_notes: HashSet<u8>,

    // List of user key press events that happened in last 500ms,
    // used for play along leeway logic
    user_pressed_recently: VecDeque<UserPress>,
}

impl PlayAlong {
    fn new(user_keyboard_range: piano_math::KeyboardRange) -> Self {
        Self {
            user_keyboard_range,
            required_notes: Default::default(),
            user_pressed_recently: Default::default(),
        }
    }

    fn update(&mut self) {
        // Instead of calling .elapsed() per item let's fetch `now` once, and substract it ourselfs
        let now = Instant::now();

        while let Some(item) = self.user_pressed_recently.front_mut() {
            let elapsed = now - item.timestamp;

            // If older than 500ms
            if elapsed.as_millis() > 500 {
                self.user_pressed_recently.pop_front();
            } else {
                // All subsequent items will by younger than front item, so we can break
                break;
            }
        }
    }

    fn user_press_key(&mut self, note_id: u8, active: bool) {
        let timestamp = Instant::now();

        if active {
            self.user_pressed_recently
                .push_back(UserPress { timestamp, note_id });
            self.required_notes.remove(&note_id);
        }
    }

    fn file_press_key(&mut self, note_id: u8, active: bool) {
        if active {
            if let Some((id, _)) = self
                .user_pressed_recently
                .iter()
                .enumerate()
                .find(|(_, item)| item.note_id == note_id)
            {
                self.user_pressed_recently.remove(id);
            } else {
                self.required_notes.insert(note_id);
            }
        } else {
            self.required_notes.remove(&note_id);
        }
    }

    pub fn press_key(&mut self, src: KeyPressSource, note_id: u8, active: bool) {
        if !self.user_keyboard_range.contains(note_id) {
            return;
        }

        match src {
            KeyPressSource::User => self.user_press_key(note_id, active),
            KeyPressSource::File => self.file_press_key(note_id, active),
        }
    }

    pub fn are_required_keys_pressed(&self) -> bool {
        self.required_notes.is_empty()
    }
}
