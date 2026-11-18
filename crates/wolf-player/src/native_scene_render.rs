//! CPU scene composition, sharing the archive/image cache with native pictures.
use std::{cell::RefCell, sync::Arc};
use wolf_media::Image;
use crate::{native_services::Services, native_scene::{Scene, SceneActorView}};

pub struct Blit {
    pub source: [u32; 4],
    pub destination: [i32; 4],
    pub opacity: u8,
    pub blend: u8,
}

/// Clipped nearest sampling with straight-alpha source-over. Special blend
/// modes operate on byte-space channels, as the native picture backend does.
pub fn blit(target: &mut Image, image: &Image, command: &Blit) -> Result<(), String> {
    let [sx, sy, sw, sh] = command.source;
    let [dx, dy, dw, dh] = command.destination;
    let (iw, ih) = image.dimensions();
    let (tw, th) = target.dimensions();
    if sw == 0 || sh == 0 || dw <= 0 || dh <= 0 || sx.checked_add(sw).is_none_or(|v|v>iw) || sy.checked_add(sh).is_none_or(|v|v>ih) {
        return Err("invalid scene blit rectangle".into());
    }
    if command.blend > 3 { return Err("unsupported actor blend".into()); }
    let left = i64::from(dx).clamp(0, i64::from(tw));
    let right = (i64::from(dx)+i64::from(dw)).clamp(0,i64::from(tw));
    let top = i64::from(dy).clamp(0,i64::from(th));
    let bottom = (i64::from(dy)+i64::from(dh)).clamp(0,i64::from(th));
    for y in top..bottom {
        let iy = u64::from(sy)+(y-i64::from(dy)) as u64*u64::from(sh)/dh as u64;
        for x in left..right {
            let ix = u64::from(sx)+(x-i64::from(dx)) as u64*u64::from(sw)/dw as u64;
            let p = &image.bytes()[((iy*u64::from(iw)+ix)*4) as usize..][..4];
            let product = u32::from(p[3])*u32::from(command.opacity);
            let a = if matches!(command.blend,1|2) {(product+127)/255} else {product/255};
            if a == 0 { continue; }
            let q = &mut target.bytes_mut()[((y*i64::from(tw)+x)*4) as usize..][..4];
            let b = u32::from(q[3]);
            let out = a*255+b*(255-a);
            for c in 0..3 {
                let s = u32::from(p[c]); let d = u32::from(q[c]);
                // 9CAC81/9CAC98 select DxLib add/subtract modes 18/19 or
                // 2/3. Alpha scales the source before saturation; clamping
                // src+dst before alpha incorrectly dims translucent lights.
                let numerator = match command.blend {
                    1=>s*a*255+d*b*255,
                    2=>(d*b*255).saturating_sub(s*a*255),
                    0=>s*a*255+d*b*(255-a),
                    3=>(s*d/255)*a*255+d*b*(255-a),
                    _=>unreachable!(),
                };
                q[c] = ((numerator+out/2)/out).min(255) as u8;
            }
            q[3] = ((out+127)/255) as u8;
        }
    }
    Ok(())
}

pub struct SceneRenderer {
    sets: Vec<wolf_format::TileSet>,
    paths: Vec<Vec<String>>,
    cache: RefCell<Option<SceneCache>>,
    viewport: [u32;2],
    directions: u8,
    patterns: u8,
}

#[derive(PartialEq, Eq)]
struct VisualState {
    map: i32,
    tileset: u32,
    tile_pixels: i32,
    dimensions: [u32;3],
    camera: [i32;2],
    loops: [bool;2],
    animation_steps: Vec<u64>,
    actors: Vec<ActorVisual>,
}
#[derive(PartialEq, Eq)]
struct ActorVisual {
    id: i32,
    position: [i32;2],
    direction: i16,
    frame: i16,
    opacity: u16,
    blend: u16,
    tile_id: i32,
    moving: bool,
    animated: bool,
    render_quarters: [i32;2],
    jump_offset_pixels: i32,
    graphic: Vec<u8>,
}
impl From<&SceneActorView<'_>> for ActorVisual {
    fn from(a: &SceneActorView<'_>) -> Self {
        Self { id:a.id,position:a.position,direction:a.direction,frame:a.frame,opacity:a.opacity,blend:a.blend,tile_id:a.tile_id,moving:a.moving,animated:a.animated,render_quarters:a.render_quarters,jump_offset_pixels:a.jump_offset_pixels,graphic:a.graphic.to_vec() }
    }
}
struct SceneCache {
    visual: VisualState,
    tiles: Option<Vec<i32>>,
    image: Arc<Image>,
}
impl SceneRenderer {
    pub fn new(services: &mut Services) -> Result<Self,String> {
        let sets = wolf_format::parse_tilesets(&services.read_resource("BasicData/TileSetData.dat")?, &Default::default()).map_err(|e|e.to_string())?;
        let paths = sets.iter().map(|set|set.images.iter().map(|v|v.text().map(str::to_owned).map_err(|e|e.to_string())).collect::<Result<Vec<_>,_>>()).collect::<Result<Vec<_>,_>>()?;
        let config = wolf_format::parse_game(&services.read_resource("BasicData/Game.dat")?, &Default::default()).map_err(|e|e.to_string())?;
        let window=crate::window_config::WindowConfig::from_words_and_ini(&config.unknown_words, "")?;
        Ok(Self { sets, paths, cache: RefCell::new(None), viewport:[window.logical_width,window.logical_height], directions:config.configuration[1], patterns:config.configuration[7] })
    }
    pub fn render(&self, scene:&Scene, tileset:u32, player:&[u8], tick:u64, services:&mut Services) -> Result<Arc<Image>,String> {
        let [width,height] = self.viewport;
        let set = self.sets.get(tileset as usize).ok_or("scene tileset missing")?;
        let paths = &self.paths[tileset as usize];
        let tile = scene.tile_pixels()?;
        let camera = scene.camera_offset();
        let [mw,mh,layers] = scene.dimensions();
        let loop_axes=scene.loop_axes();
        let mut actors:Vec<_>=scene.visible_events().collect();
        actors.push(scene.player_view(player));
        actors.sort_by_key(|actor|(actor.position[1],actor.id));
        let visual = VisualState {
            map:scene.map_id(), tileset, tile_pixels:tile, dimensions:[mw,mh,layers], camera, loops:loop_axes,
            animation_steps: if scene.tiles().is_some() { paths.iter().skip(1).map(|path| if path.is_empty() {0} else {tick / crate::native_tile_render::animation_period(path)}).collect() } else {Vec::new()},
            actors: actors.iter().map(ActorVisual::from).collect(),
        };
        // Both VM capture commands and presentation ask for this scene. Reuse
        // only when every visible input and the actual tile array match.
        if let Some(cached) = self.cache.borrow().as_ref() {
            if cached.visual == visual && cached.tiles.as_deref() == scene.tiles() {
                return Ok(cached.image.clone());
            }
        }
        let mut target = Image::new(width as usize,height as usize,vec![0;width as usize*height as usize*4])?;
        let mut passes: [Vec<(i32,crate::native_tile_render::TileDraw)>;3] = Default::default();
        if let Some(tiles) = scene.tiles() {
            let left=camera[0].div_euclid(tile); let top=camera[1].div_euclid(tile);
            let right=(camera[0]+width as i32-1).div_euclid(tile);
            let bottom=(camera[1]+height as i32-1).div_euclid(tile);
            for screen_x in left..=right { for screen_y in top..=bottom {
                if (!loop_axes[0] && !(0..mw as i32).contains(&screen_x)) || (!loop_axes[1] && !(0..mh as i32).contains(&screen_y)) {continue;}
                let x=screen_x.rem_euclid(mw as i32) as u32;
                let y=screen_y.rem_euclid(mh as i32) as u32;
                let px=screen_x*tile-camera[0]; let py=screen_y*tile-camera[1];
                let mut priority=0;
                for layer in 0..layers {
                    let index=((layer*mw+x)*mh+y) as usize;
                    let word=*tiles.get(index).ok_or("scene tile index exceeds map")?;
                    let draws=crate::native_tile_render::resolve(word,paths,&set.flags,tile as u32,tick,|path|Ok(services.image_resource(path)?.dimensions()))?;
                    for mut draw in draws {
                        priority=priority.max(draw.priority);
                        draw.destination[0]+=px; draw.destination[1]+=py;
                        passes[usize::from(priority)].push((py+tile,draw));
                    }
                }
            }}
        }
        let draw_tile=|target:&mut Image,draw:&crate::native_tile_render::TileDraw,services:&mut Services|->Result<(),String>{
            let image=services.image_resource(&draw.path)?;
            blit(target,&image,&Blit{source:draw.source,destination:draw.destination,opacity:255,blend:0})
        };
        for (_,draw) in &passes[0] {draw_tile(&mut target,draw,services)?;}
        // A3EC76/9D7360 merges row markers with actors. At an equal foot
        // depth actors precede the row marker (9D7437..9D7485).
        passes[1].sort_by_key(|(depth,_)|*depth);
        let mut actor_index=0;
        for (depth,draw) in &passes[1] {
            while actor_index<actors.len() && actors[actor_index].position[1]*tile/2+tile-camera[1]<=*depth {
                self.actor(&mut target,&actors[actor_index],&paths[0],tile,camera,services)?;
                actor_index+=1;
            }
            draw_tile(&mut target,draw,services)?;
        }
        for actor in &actors[actor_index..] {self.actor(&mut target,actor,&paths[0],tile,camera,services)?;}
        for (_,draw) in &passes[2] {draw_tile(&mut target,draw,services)?;}
        let image = Arc::new(target);
        self.cache.replace(Some(SceneCache { visual, tiles:scene.tiles().map(<[i32]>::to_vec), image:image.clone() }));
        Ok(image)
    }
    fn actor(&self,target:&mut Image,actor:&SceneActorView<'_>,base:&str,tile:i32,camera:[i32;2],services:&mut Services)->Result<(),String>{
        let graphic=std::str::from_utf8(actor.graphic.strip_suffix(&[0]).unwrap_or(actor.graphic)).map_err(|_|"actor graphic is not UTF-8")?;
        if actor.opacity==0 || (actor.tile_id<0 && graphic.is_empty()) {return Ok(());}
        let path=if actor.tile_id>=0 {base} else {graphic};
        let image=services.image_resource(path)?;
        let (w,h)=image.dimensions();
        let layout=crate::native_actor_render::layout(crate::native_actor_render::ActorSpec {graphic:actor.graphic,tile_id:actor.tile_id,direction:actor.direction,frame:actor.frame,animated:actor.animated},[w,h],tile as u32,self.directions,self.patterns)?;
        // Scene stores logical half-tile coordinates, while movement routes
        // retain the native quarter-pixel render position during interpolation.
        // Use that render position when available so player takeover and route
        // motion remain smooth at the fixed presentation rate.
        let x=actor.render_quarters[0]/4+tile/2-camera[0]-layout.origin[0];
        let y=actor.render_quarters[1]/4+tile-camera[1]-layout.origin[1]-actor.jump_offset_pixels;
        blit(target,&image,&Blit{source:layout.source,destination:[x,y,layout.size[0] as i32,layout.size[1] as i32],opacity:actor.opacity.min(255) as u8,blend:u8::try_from(actor.blend).map_err(|_|"actor blend overflow")?})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translucent_actor_light_scales_before_additive_saturation(){
        let source=Image::new(1,1,vec![200,100,20,128]).unwrap();
        let mut target=Image::new(1,1,vec![200,50,250,255]).unwrap();
        blit(&mut target,&source,&Blit{source:[0,0,1,1],destination:[0,0,1,1],opacity:255,blend:1}).unwrap();
        assert_eq!(target.bytes(),[255,100,255,255]);
        let mut target=Image::new(1,1,vec![80,100,3,255]).unwrap();
        blit(&mut target,&source,&Blit{source:[0,0,1,1],destination:[0,0,1,1],opacity:255,blend:2}).unwrap();
        assert_eq!(target.bytes(),[0,50,0,255]);
    }
    #[test]
    fn transparent_scene_preserves_source_color_and_clips_atlas() {
        let mut target=Image::new(2,1,vec![0;8]).unwrap();
        let source=Image::new(2,1,vec![255,0,0,255,20,80,160,128]).unwrap();
        blit(&mut target,&source,&Blit{source:[1,0,1,1],destination:[-1,0,2,1],opacity:255,blend:0}).unwrap();
        assert_eq!(target.bytes(),[20,80,160,128,0,0,0,0]);
    }
}
