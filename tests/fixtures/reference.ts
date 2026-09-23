// Reference data shaped exactly like our instance's v1 responses (people anonymised).
import type { FetchStub } from '../helpers/fetch-stub.js'

export const users = [
  { ResourceType: 'User', Id: 1, FirstName: 'Administrator', LastName: 'Administrator', Email: 'admin@nonexistingemail.com', Login: 'admin', DeleteDate: null, IsActive: false, Kind: 'User' },
  { ResourceType: 'User', Id: 2286, FirstName: 'Foxxo', LastName: 'Vulpes', Email: 'foxxo@example.com', Login: 'fvulpes', DeleteDate: null, IsActive: true, Kind: 'User' },
  { ResourceType: 'User', Id: 16, FirstName: 'Giorgio', LastName: 'Verdi', Email: 'giorgio@example.com', Login: 'giorgio@example.com', DeleteDate: null, IsActive: true, Kind: 'User' },
  { ResourceType: 'User', Id: 17, FirstName: 'Giorgia', LastName: 'Rossi', Email: 'giorgia@example.com', Login: 'grossi', DeleteDate: null, IsActive: true, Kind: 'User' },
  { ResourceType: 'User', Id: 2429, FirstName: 'Rocco', LastName: 'Neri', Email: 'rocco@example.com', Login: 'rocco@example.com', DeleteDate: null, IsActive: false, Kind: 'User' },
]

export const roles = [
  { ResourceType: 'Role', Id: 1, Name: 'Backend Developer', HasEffort: true },
  { ResourceType: 'Role', Id: 12, Name: 'Designer', HasEffort: true },
  { ResourceType: 'Role', Id: 13, Name: 'Developer', HasEffort: true },
  { ResourceType: 'Role', Id: 11, Name: 'Frontend Developer', HasEffort: true },
  { ResourceType: 'Role', Id: 7, Name: 'Product Owner', HasEffort: true },
  { ResourceType: 'Role', Id: 10, Name: 'Tester', HasEffort: true },
  { ResourceType: 'Role', Id: 8, Name: 'Top Manager', HasEffort: false },
]

export const teams = [
  { ResourceType: 'Team', Id: 447, Name: 'Core Team', IsActive: true },
  { ResourceType: 'Team', Id: 19523, Name: 'Pandolfo Team', IsActive: true },
  { ResourceType: 'Team', Id: 300, Name: 'Legacy Team', IsActive: false },
]

export const projects = [
  { ResourceType: 'Project', Id: 179, Name: 'Activity Manager', IsActive: false, Abbreviation: 'AMP', Process: { ResourceType: 'Process', Id: 5, Name: 'Scrum Emme 4' } },
  { ResourceType: 'Project', Id: 26080, Name: 'SBP', IsActive: true, Abbreviation: 'SBP', Process: { ResourceType: 'Process', Id: 13, Name: 'Mamami 2025' } },
]

const wf = (Id: number, ParentWorkflow: unknown = null) => ({ ResourceType: 'Workflow', Id, Name: 'Project workflow', ParentWorkflow })
const role = (Id: number, Name: string) => ({ ResourceType: 'Role', Id, Name })
const state = (Id: number, Name: string, NumericPriority: number, extra: Record<string, unknown> = {}) => ({
  ResourceType: 'EntityState',
  Id,
  Name,
  IsInitial: false,
  IsFinal: false,
  IsPlanned: false,
  NumericPriority,
  Workflow: wf(202),
  ParentEntityState: null,
  Role: role(13, 'Developer'),
  EntityType: { ResourceType: 'EntityType', Id: 4, Name: 'UserStory' },
  ...extra,
})

/** UserStory states of process 13, plus one team sub-workflow state for the flagging logic. */
export const storyStates = [
  state(681, 'Open', 1, { IsInitial: true, Role: role(7, 'Product Owner') }),
  state(682, 'Refining', 3.25, { Role: role(7, 'Product Owner') }),
  state(684, 'Designable', 3.8125, { Role: role(12, 'Designer') }),
  state(683, 'Estimated', 3.90625),
  state(685, 'Ready', 4),
  state(686, 'In Progress', 5),
  state(687, 'Coded', 6),
  state(688, 'Testing', 7),
  state(689, 'Deliverable', 7.5, { Role: role(7, 'Product Owner') }),
  state(690, 'Done', 8, { IsFinal: true, Role: null }),
  state(900, 'Code Review', 5.5, {
    Workflow: { ResourceType: 'Workflow', Id: 950, Name: 'Core Team flow', ParentWorkflow: { ResourceType: 'Workflow', Id: 202 } },
    ParentEntityState: { ResourceType: 'EntityState', Id: 686, Name: 'In Progress' },
  }),
]

export const taskStates = [
  { ...state(691, 'Open', 1, { IsInitial: true }), EntityType: { ResourceType: 'EntityType', Id: 5, Name: 'Task' }, Workflow: wf(203) },
  { ...state(692, 'In Progress', 2), EntityType: { ResourceType: 'EntityType', Id: 5, Name: 'Task' }, Workflow: wf(203) },
  { ...state(693, 'Coded', 3), EntityType: { ResourceType: 'EntityType', Id: 5, Name: 'Task' }, Workflow: wf(203) },
  { ...state(694, 'Done', 4, { IsFinal: true }), EntityType: { ResourceType: 'EntityType', Id: 5, Name: 'Task' }, Workflow: wf(203) },
]

const cf = (Id: number, Name: string, FieldType: string, Value: string | null) => ({
  ResourceType: 'CustomField',
  Id,
  Name,
  Value,
  FieldType,
  Required: false,
  EntityType: { ResourceType: 'EntityType', Id: 4, Name: 'UserStory' },
  Process: { ResourceType: 'Process', Id: 13, Name: 'Mamami 2025' },
})

export const storyCustomFields = [
  cf(140, 'Specification', 'URL', null),
  cf(150, 'BackEnd', 'DropDown', 'To Do\r\nDoing\r\nReview\r\nDone'),
  cf(152, 'FrontEnd', 'DropDown', 'To Do\r\nDoing\r\nReview\r\nDone'),
  cf(153, 'Figma', 'URL', null),
  cf(159, 'Total Hours', 'Number', null),
  cf(160, 'Release Instructions', 'RichText', null),
]

export const loggedUser = { ResourceType: 'User', Id: 2286, FirstName: 'Foxxo', LastName: 'Vulpes', Login: 'fvulpes', IsActive: true, Kind: 'User' }

export function general(id: number, entityType: string, entityTypeId = 4, project = { Id: 26080, Name: 'SBP', processId: 13 }) {
  return {
    ResourceType: 'General',
    Id: id,
    Name: `Card ${id}`,
    EntityType: { ResourceType: 'EntityType', Id: entityTypeId, Name: entityType },
    Project: { ResourceType: 'Project', Id: project.Id, Name: project.Name, Process: { ResourceType: 'Process', Id: project.processId, Name: 'Mamami 2025' } },
  }
}

/** Registers the reference-data endpoints the resolvers read. */
export function withReferenceData(stub: FetchStub): FetchStub {
  const where = (text: string) => (q: URLSearchParams) => (q.get('where') ?? '').includes(text)
  return stub
    .get('/api/v1/Users', { Items: users })
    .get('/api/v1/Users/LoggedUser', loggedUser)
    .get('/api/v1/Roles', { Items: roles })
    .get('/api/v1/Teams', { Items: teams })
    .get('/api/v1/Projects', { Items: projects })
    .get('/api/v1/EntityStates', { Items: storyStates }, { query: where("EntityType.Name eq 'UserStory'") })
    .get('/api/v1/EntityStates', { Items: taskStates }, { query: where("EntityType.Name eq 'Task'") })
    .get('/api/v1/EntityStates', { Items: [...storyStates, ...taskStates] }, { query: (q) => !q.get('where') })
    .get('/api/v1/EntityStates', { Items: [] })
    .get('/api/v1/CustomFields', { Items: storyCustomFields }, { query: where("EntityType.Name eq 'UserStory'") })
    .get('/api/v1/CustomFields', { Items: [] })
}
