// Virtual methods (IEEE 1800-2017 8.20): a call through a base-class handle runs the
// implementation of the object's class (the nearest override), also from inside a base
// method; `super.m()` and non-virtual methods are bound by the handle's class.
class Animal;
  virtual function string sound();
    return "...";
  endfunction
  function string kind();
    return "animal";
  endfunction
  function string speak();
    return sound();  // virtual: the object's class decides
  endfunction
endclass

class Dog extends Animal;
  function string sound();
    return "woof";
  endfunction
  function string kind();
    return "dog";
  endfunction
endclass

class Puppy extends Dog;
  function string sound();
    return {"small ", super.sound()};
  endfunction
endclass

class Cat extends Animal;
endclass

module m;
  Animal x;
  string got;
  initial begin
    x = new;
    if (x.speak() != "...") $display("FAILED -- animal");
    begin
      Dog d = new;
      Puppy p = new;
      Cat c = new;
      x = d;
      if (x.sound() != "woof" || x.kind() != "animal" || x.speak() != "woof") $display("FAILED -- dog");
      x = p;
      if (x.sound() != "small woof" || x.speak() != "small woof") $display("FAILED -- puppy %s", x.sound());
      x = c;
      if (x.sound() != "...") $display("FAILED -- cat");
      if (d.kind() != "dog") $display("FAILED -- non-virtual through the derived handle");
    end
    $display("PASSED");
  end
endmodule
