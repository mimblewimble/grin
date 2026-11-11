// Copyright 2026 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::tor::{Error, ServiceStatus, SERVICE_STATUS};

use futures::StreamExt;
use std::time::{Duration, Instant};
use tor_hsservice::status::{Problem as OnionServiceProblem, State as OnionServiceState};
use tor_hsservice::RunningOnionService;
use tor_rtcompat::{Runtime, SleepProviderExt, SpawnExt};

const STATUS_WARNING_DELAY: Duration = Duration::from_secs(120);
const STATUS_WARNING_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Default)]
struct StatusLog {
	since: Option<Instant>,
	last_warning: Option<Instant>,
	problem: Option<String>,
	broken: bool,
}

impl StatusLog {
	fn update(
		&mut self,
		state: OnionServiceState,
		problem: Option<String>,
		now: Instant,
	) -> Option<(log::Level, Duration)> {
		if matches!(
			state,
			OnionServiceState::Running | OnionServiceState::Shutdown
		) {
			let recovery = self
				.last_warning
				.and(self.since)
				.filter(|_| state == OnionServiceState::Running)
				.map(|since| (log::Level::Info, now.duration_since(since)));
			*self = Self::default();
			return recovery;
		}
		if let Some(problem) = problem {
			self.problem = Some(problem);
		}
		// Ignore normal descriptor refreshes
		if state == OnionServiceState::Bootstrapping
			&& self.problem.is_none()
			&& self.since.is_none()
		{
			return None;
		}
		let elapsed = now.duration_since(*self.since.get_or_insert(now));
		let broken = state == OnionServiceState::Broken;
		let report = (broken && !self.broken)
			|| match self.last_warning {
				Some(last) => now.duration_since(last) >= STATUS_WARNING_INTERVAL,
				None => elapsed >= STATUS_WARNING_DELAY,
			};
		self.broken = broken;
		if report {
			self.last_warning = Some(now);
			Some((
				if broken {
					log::Level::Error
				} else {
					log::Level::Warn
				},
				elapsed,
			))
		} else {
			None
		}
	}
}

pub(super) fn spawn_status_logger<R>(
	runtime: &R,
	service: &RunningOnionService,
	onion_address: String,
) -> Result<(), Error>
where
	R: Runtime,
{
	let service_status = SERVICE_STATUS.lock().unwrap().get(&onion_address).cloned();
	let mut statuses = service.status_events();

	let mut status = service.status();
	debug!("Onion service status at http://{onion_address}.onion: {status:?}");

	let timer = runtime.clone();
	runtime
		.spawn(async move {
			let mut monitor = StatusLog::default();
			loop {
				let state = status.state();
				let problem = status.current_problem().and_then(onion_service_problem);
				if let Some(ref service_status) = service_status {
					update_status(&mut service_status.lock().unwrap(), state, problem.as_deref());
				}
				if let Some((level, elapsed)) = monitor.update(state, problem, Instant::now()) {
					if level == log::Level::Info {
						info!("Onion service recovered after {}s at http://{onion_address}.onion", elapsed.as_secs());
					} else {
						log::log!(level, "Onion service reports {state:?} after {}s of problems at http://{onion_address}.onion; latest problem: {}",
							elapsed.as_secs(), monitor.problem.as_deref().unwrap_or("no details reported"));
					}
				}
				// Keep checking while the status stays unchanged
				match timer.timeout(Duration::from_secs(30), statuses.next()).await {
					Ok(Some(next)) => {
						if format!("{next:?}") != format!("{status:?}") {
							debug!("Onion service status at http://{onion_address}.onion: {next:?}");
						}
						status = next;
					}
					Ok(None) => break,
					Err(_) => {}
				}
			}
		})
		.map_err(|error| {
			Error::Generic(format!("Could not monitor Onion service: {error}"))
		})?;
	Ok(())
}

fn update_status(status: &mut ServiceStatus, state: OnionServiceState, problem: Option<&str>) {
	use OnionServiceState::*;

	*status = match state {
		Running => ServiceStatus::Running,
		DegradedReachable => ServiceStatus::Degraded,
		Recovering => ServiceStatus::Recovering,
		Bootstrapping => {
			//TODO: Currently arti is not showing right status when publisher still has
			// Bootstrapping status, so we set Running after Recovering.
			if matches!(status, ServiceStatus::Recovering) {
				ServiceStatus::Running
			} else {
				ServiceStatus::Recovering
			}
		}
		Broken | DegradedUnreachable => ServiceStatus::Errored(Error::Generic(
			problem
				.map(str::to_owned)
				.unwrap_or_else(|| format!("Onion service is {:?}", state)),
		)),
		//TODO: arti is sending shutdown first.
		Shutdown if !matches!(status, ServiceStatus::Starting) => ServiceStatus::Stopped,
		_ => ServiceStatus::Starting,
	};
}

fn onion_service_problem(problem: &OnionServiceProblem) -> Option<String> {
	match problem {
		OnionServiceProblem::DescriptorUpload(errors) if errors.is_empty() => None,
		OnionServiceProblem::Ipt(errors) if errors.is_empty() => None,
		_ => Some(format!("{problem:?}")),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use OnionServiceState::{
		Bootstrapping, Broken, DegradedReachable, DegradedUnreachable, Recovering, Running,
		Shutdown,
	};

	#[test]
	fn service_states() {
		let mut status = ServiceStatus::Starting;
		update_status(&mut status, Bootstrapping, None);
		assert!(matches!(status, ServiceStatus::Recovering));
		update_status(&mut status, Running, None);
		assert!(matches!(status, ServiceStatus::Running));
		update_status(&mut status, DegradedReachable, None);
		assert!(matches!(status, ServiceStatus::Degraded));
		update_status(&mut status, Bootstrapping, None);
		assert!(matches!(status, ServiceStatus::Recovering));
		update_status(&mut status, Recovering, None);
		assert!(matches!(status, ServiceStatus::Recovering));
		update_status(&mut status, DegradedUnreachable, Some("upload failed"));
		assert!(
			matches!(status, ServiceStatus::Errored(Error::Generic(ref e)) if e == "upload failed")
		);
		update_status(&mut status, Running, None);
		assert!(matches!(status, ServiceStatus::Running));
		update_status(&mut status, Shutdown, None);
		assert!(matches!(status, ServiceStatus::Stopped));
	}

	#[test]
	fn brief_problem() {
		let mut monitor = StatusLog::default();
		let now = Instant::now();
		assert_eq!(monitor.update(Bootstrapping, None, now), None);
		assert_eq!(monitor.update(Running, None, now), None);
		assert_eq!(
			monitor.update(DegradedReachable, Some("upload timeout".into()), now),
			None
		);
		assert_eq!(
			monitor.update(Running, None, now + Duration::from_secs(60)),
			None
		);
		assert!(monitor.problem.is_none());
	}

	#[test]
	fn warning_and_recovery() {
		let mut monitor = StatusLog::default();
		let now = Instant::now();
		let state = DegradedUnreachable;
		assert_eq!(
			monitor.update(state, Some("upload timeout".into()), now),
			None
		);
		// Keep the timer across retries
		assert_eq!(
			monitor.update(Bootstrapping, None, now + Duration::from_secs(60)),
			None
		);
		assert_eq!(
			monitor.update(state, None, now + STATUS_WARNING_DELAY),
			Some((log::Level::Warn, STATUS_WARNING_DELAY))
		);
		assert_eq!(monitor.problem.as_deref(), Some("upload timeout"));
		assert_eq!(
			monitor.update(state, None, now + Duration::from_secs(150)),
			None
		);
		let repeat = STATUS_WARNING_DELAY + STATUS_WARNING_INTERVAL;
		assert_eq!(
			monitor.update(state, None, now + repeat),
			Some((log::Level::Warn, repeat))
		);
		assert_eq!(
			monitor.update(Running, None, now + repeat),
			Some((log::Level::Info, repeat))
		);
		assert_eq!(monitor.update(Running, None, now + repeat), None);
	}

	#[test]
	fn broken_and_shutdown() {
		let mut monitor = StatusLog::default();
		let now = Instant::now();
		assert_eq!(monitor.update(Recovering, None, now), None);
		assert_eq!(
			monitor.update(Broken, Some("runtime failure".into()), now),
			Some((log::Level::Error, Duration::ZERO))
		);
		assert_eq!(monitor.update(Broken, None, now), None);
		assert_eq!(monitor.update(Shutdown, None, now), None);
		assert!(monitor.since.is_none());
	}

	#[test]
	fn retry_without_details() {
		let mut monitor = StatusLog::default();
		let now = Instant::now();
		assert_eq!(monitor.update(Recovering, None, now), None);
		assert_eq!(
			monitor.update(Bootstrapping, None, now + STATUS_WARNING_DELAY),
			Some((log::Level::Warn, STATUS_WARNING_DELAY))
		);
	}

	#[test]
	fn empty_errors() {
		assert!(onion_service_problem(&OnionServiceProblem::DescriptorUpload(vec![])).is_none());
		assert!(onion_service_problem(&OnionServiceProblem::Ipt(vec![])).is_none());
	}
}
