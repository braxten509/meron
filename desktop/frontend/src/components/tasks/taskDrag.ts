// Defines the drag behavior shared by desktop task controls. The transferred
// identity determines which task or list a drop concerns.
// Callers include TasksPanel.tsx and ConversationTabs.tsx.

import { MouseSensor, TouchSensor, useSensor, useSensors } from '@dnd-kit/core'

function isControl(target: EventTarget | null) {
  return target instanceof Element && target.closest('[data-no-drag]') !== null
}

class TaskMouseSensor extends MouseSensor {
  static activators: typeof MouseSensor.activators = MouseSensor.activators.map((activator) => ({
    ...activator,
    handler: (event, options) => !isControl(event.target) && activator.handler(event, options),
  }))
}

class TaskTouchSensor extends TouchSensor {
  static activators: typeof TouchSensor.activators = TouchSensor.activators.map((activator) => ({
    ...activator,
    handler: (event, options) => !isControl(event.target) && activator.handler(event, options),
  }))
}

export function useTaskDragSensors() {
  return useSensors(
    useSensor(TaskMouseSensor, { activationConstraint: { distance: 6 } }),
    // A swipe scrolls normally; holding still starts a reorder gesture.
    useSensor(TaskTouchSensor, { activationConstraint: { delay: 250, tolerance: 5 } }),
  )
}
