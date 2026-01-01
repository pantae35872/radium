pub struct FrameTracker {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    stride: usize,
    width: usize,
    height: usize,
}

impl FrameTracker {
    pub fn new(width: usize, height: usize, stride: usize) -> Self {
        Self { min_x: width - 1, min_y: height - 1, max_x: 0, max_y: 0, stride, width, height }
    }

    pub fn track(&mut self, x: usize, y: usize) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    pub fn track_all(&mut self) {
        self.min_x = 0;
        self.min_y = 0;
        self.max_x = self.width - 1;
        self.max_y = self.height - 1;
    }

    pub fn frame_buffer_min(&self) -> usize {
        self.min_y * self.stride + self.min_x
    }

    pub fn frame_buffer_max(&self) -> usize {
        self.max_y * self.stride + self.max_x
    }

    pub fn frame_width(&self) -> usize {
        self.max_x.abs_diff(self.min_x) + 1
    }

    pub fn frame_height(&self) -> usize {
        self.max_y.abs_diff(self.min_y) + 1
    }

    pub fn reset(&mut self) {
        self.min_x = self.width - 1;
        self.min_y = self.height - 1;
        self.max_x = 0;
        self.max_y = 0;
    }
}
