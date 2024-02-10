FS_LOAD_PATHS = Fs.get_load_paths
FS_LOADED_PATHS = []
module Require
  # TODO: private opt
  def load(file, priv=false)
    find_path = file
    script = file_path = nil
    FS_LOAD_PATHS.each do |load_path|
      file_path = File.join(load_path, find_path)
      break if (script = Fs.get_file_from_fs(file_path))
    end
    eval_or_require_extension(script, file_path, file, force: true)
  rescue LoadError => e
    find_path = file
    puts "load local #{find_path}"
    Kernel.require(find_path)
  rescue SyntaxError => e
    puts e.message
  end
  def require(file)
    find_path = file
    script = file_path = nil
    if File.extname(file) == ''
      ['.rb', '.so'].each do |ext|
        find_path = file + ext
        FS_LOAD_PATHS.each do |load_path|
          file_path = File.join(load_path, find_path)
          break if (script = Fs.get_file_from_fs(file_path))
        end
        break if script
      end
    else
      if File.absolute_path?(find_path)
        file_path = find_path
        script = Fs.get_file_from_fs(file_path)
      else
        FS_LOAD_PATHS.each do |load_path|
          file_path = File.join(load_path, find_path)
          break if (script = Fs.get_file_from_fs(file_path))
        end
      end
    end
    eval_or_require_extension(script, file_path, file)
  rescue LoadError => e
    find_path = file
    puts "require local #{find_path}"
    Kernel.require(find_path)
  rescue SyntaxError => e
    puts e.message
  end
  def require_relative(file)
    find_path = file
    script = file_path = nil
    call_dir = File.dirname(caller_locations(1, 1).first.absolute_path)
    if File.extname(file) == ''
      ['.rb', '.so'].each do |ext|
        find_path = file + ext
        file_path = File.expand_path(File.join(call_dir, find_path))
        break if (script = Fs.get_file_from_fs(file_path))
      end
    else
      file_path = File.expand_path(File.join(call_dir, find_path))
      script = Fs.get_file_from_fs(file_path)
    end
    eval_or_require_extension(script, file_path, file)
  rescue LoadError => e
    find_path = file
    file_path = File.expand_path(File.join(call_dir, find_path))
    puts "require_relative local #{file_path}"
    Kernel.require_relative(file_path)
  rescue SyntaxError => e
    puts e.message
  end
  def eval_or_require_extension(script, file_path, file, force: false)
    if script.nil?
      raise LoadError, "cannot load such file -- #{file}"
    else
      if !force && FS_LOADED_PATHS.include?(file_path)
        return false
      end
      FS_LOADED_PATHS << file_path
      FS_LOADED_PATHS.uniq!
      if File.extname(file_path) == '.rb'
        RubyVM::InstructionSequence.compile(script, File.basename(file_path), file_path).eval
        return true
      else
        puts "require native extension #{File.basename(file_path)}"
        Kernel.require(File.basename(file_path))
      end
    end
  end
end
include Require
RubyVM::InstructionSequence.compile(Fs.get_start_file_script, File.basename(Fs.get_start_file_name), Fs.get_start_file_name).eval
