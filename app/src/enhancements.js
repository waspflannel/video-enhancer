// Components describe adjustments; the existing pipeline still renders one combined job.
globalThis.Enhancements = (() => {
  const defaults = {
    scale: 1, quality: 4, upscale_method: 'vsr', upscale_strength: 1,
    target_fps: '', frame_quality: 1, detect_scene_changes: true,
    output_encoding: 'h264', encoder_preset: 4, encoder_quality: 19,
    denoise: 0, denoise_quality: 0, deblur: 0, deblur_quality: 0,
    contrast: 1, saturation: 1, vibrance: 0, exposure: 0, warmth: 0, sharpening: 0,
    temporal_denoise: '', portrait_mode: 'off', segmentation_mode: 0,
    blur_strength: 0.5, background_color: '#00b140',
    relighting_mode: 'off', hdri: null, relighting_quality: 1, relighting_pan: 0,
    relighting_field_of_view: 60, relighting_foreground_gain: 1, relighting_background_gain: 1,
    relighting_specularity: 0, relighting_blur_strength: 0.5, environment_background: false,
    hdr_enabled: false, hdr_contrast: 100, hdr_saturation: 100, hdr_middle_gray: 50,
    hdr_max_luminance: 650, hdr_debanding: true,
  };
  const groups = [
    { id: 'upscale', label: 'Upscaling detail', fields: ['quality', 'upscale_method', 'upscale_strength'] },
    { id: 'denoise', label: 'Noise reduction', fields: ['denoise', 'denoise_quality'] },
    { id: 'deblur', label: 'Deblur', fields: ['deblur', 'deblur_quality'] },
    { id: 'sharpen', label: 'Sharpening', fields: ['sharpening'] },
    { id: 'contrast', label: 'Contrast', fields: ['contrast'] },
    { id: 'vibrance', label: 'Vibrance', fields: ['vibrance'] },
    { id: 'saturation', label: 'Saturation', fields: ['saturation'] },
    { id: 'exposure', label: 'Exposure', fields: ['exposure'] },
    { id: 'warmth', label: 'Warmth', fields: ['warmth'] },
    { id: 'temporal', label: 'Temporal denoising', fields: ['temporal_denoise'] },
    { id: 'hdr', label: 'SDR to HDR', fields: ['hdr_enabled', 'hdr_contrast', 'hdr_saturation', 'hdr_middle_gray', 'hdr_max_luminance', 'hdr_debanding'] },
    { id: 'portrait', label: 'Portrait background', fields: ['portrait_mode', 'segmentation_mode', 'blur_strength', 'background_color'] },
    { id: 'relighting', label: 'Portrait relighting', fields: ['relighting_mode', 'hdri', 'relighting_quality', 'relighting_pan', 'relighting_field_of_view', 'relighting_foreground_gain', 'relighting_background_gain', 'relighting_specularity', 'relighting_blur_strength', 'environment_background'] },
  ];
  const builtins = [
    { name: 'Clean footage', settings: { quality: 19 } },
    { name: 'Clean + colour', settings: { contrast: 1.08, vibrance: 0.3, saturation: 1.06, sharpening: 0.15 } },
    { name: 'Compressed footage', settings: { quality: 4 } },
    { name: 'Soft footage', settings: { deblur: 0.15, deblur_quality: 1 } },
    { name: 'Noisy footage', settings: { denoise: 0.2, denoise_quality: 1 } },
  ];
  const fields = new Set(groups.flatMap(group => group.fields));
  const choices = {
    quality: [0, 1, 2, 3, 4, 16, 17, 18, 19, 21, 23], upscale_method: ['vsr', 'lightweight'],
    denoise_quality: [0, 1, 2, 3], deblur_quality: [0, 1, 2, 3], temporal_denoise: ['', 0, 1],
    portrait_mode: ['off', 'blur', 'replace', 'mask'], segmentation_mode: [0, 1, 2, 3],
    relighting_mode: ['off', 'relighting', 'aigs'], relighting_quality: [0, 1, 2, 3],
  };
  const ranges = {
    upscale_strength: [0, 1], denoise: [0, 1], deblur: [0, 1], sharpening: [0, 2],
    contrast: [0.5, 1.5], vibrance: [-1, 1], saturation: [0, 2], exposure: [-2, 2], warmth: [-1, 1],
    blur_strength: [0, 1], relighting_pan: [-180, 180], relighting_field_of_view: [1, 179],
    relighting_foreground_gain: [0, 4], relighting_background_gain: [0, 4],
    relighting_specularity: [0, 1], relighting_blur_strength: [0, 2],
    hdr_contrast: [0, 200], hdr_saturation: [0, 200], hdr_middle_gray: [10, 100], hdr_max_luminance: [400, 2000],
  };

  function sanitizeTemplate(value) {
    if (!value || typeof value.name !== 'string' || !value.name.trim() || value.name.trim().length > 120) {
      throw new Error('Give the enhancement a name of 1–120 characters.');
    }
    if (!value.settings || typeof value.settings !== 'object' || Array.isArray(value.settings) || !Object.keys(value.settings).length) {
      throw new Error('Add at least one adjustment before saving an enhancement.');
    }
    const settings = {};
    for (const [field, setting] of Object.entries(value.settings)) {
      if (!fields.has(field)) throw new Error(`Unsupported enhancement setting: ${field}.`);
      let valid;
      if (choices[field]) valid = choices[field].includes(setting);
      else if (ranges[field]) {
        const [min, max] = ranges[field];
        valid = typeof setting === 'number' && Number.isFinite(setting) && setting >= min && setting <= max;
        if (field.startsWith('hdr_')) valid &&= Number.isInteger(setting);
      } else if (field === 'hdri') valid = setting === null || (typeof setting === 'string' && setting.length <= 32768);
      else if (field === 'background_color') valid = typeof setting === 'string' && /^#[\da-f]{6}$/i.test(setting);
      else valid = typeof setting === 'boolean' && typeof defaults[field] === 'boolean';
      if (!valid) throw new Error(`Unsupported enhancement setting: ${field}.`);
      settings[field] = setting;
    }
    return { name: value.name.trim(), settings };
  }

  function compose(components) {
    const settings = { ...defaults };
    for (const component of components) if (component.enabled) Object.assign(settings, component.settings);
    return settings;
  }

  function overlaps(components) {
    const claimed = new Set(), overridden = Object.create(null);
    for (const component of [...components].reverse()) {
      overridden[component.id] = [];
      if (!component.enabled) continue;
      for (const field of Object.keys(component.settings)) {
        if (claimed.has(field)) overridden[component.id].push(field);
        claimed.add(field);
      }
    }
    return overridden;
  }

  return { defaults, groups, builtins, choices, ranges, compose, overlaps, sanitizeTemplate };
})();
