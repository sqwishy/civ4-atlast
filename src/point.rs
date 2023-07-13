use image::{Rgba, RgbaImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

impl Point {
    // pub fn from_xy((x, y): (u32, u32)) -> Self {
    //     Self { x, y }
    // }

    pub fn checked_add(&self, Point { x, y }: Point) -> Option<Self> {
        Some(Point {
            x: self.x.checked_add(x)?,
            y: self.y.checked_add(y)?,
        })
    }

    pub fn next_x(&self) -> Option<Self> {
        self.x.checked_add(1).map(|x| Point { x, y: self.y })
    }

    // pub fn next_y(&self) -> Option<Self> {
    //     self.y.checked_add(1).map(|y| Point { x: self.x, y })
    // }

    /// yield this point and subsequent points incrementing x for each
    pub fn x_counter(&self) -> impl Iterator<Item = Point> {
        let y = self.y;
        (self.x..=u32::MAX).map(move |x| Point { x, y })
    }

    /// yield this point and subsequent points incrementing y for each
    pub fn y_counter(&self) -> impl Iterator<Item = Point> {
        let x = self.x;
        (self.y..=u32::MAX).map(move |y| Point { x, y })
    }

    pub fn get_pixel_at(self, buf: &RgbaImage) -> Option<(&'_ Rgba<u8>, Self)> {
        buf.get_pixel_checked(self.x, self.y)
            .map(|pixel| (pixel, self))
    }

    pub fn scan_x(self, buf: &RgbaImage) -> impl Iterator<Item = (&'_ Rgba<u8>, Self)> {
        self.x_counter().map_while(|at| at.get_pixel_at(buf))
    }

    pub fn scan_y(self, buf: &RgbaImage) -> impl Iterator<Item = (&'_ Rgba<u8>, Self)> {
        self.y_counter().map_while(|at| at.get_pixel_at(buf))
    }
}
