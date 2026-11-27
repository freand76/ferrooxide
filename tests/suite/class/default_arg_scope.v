// The default value of an argument is evaluated in the scope of the subroutine's
// declaration (IEEE 1800-2017 13.5.3): in a method, `this` is the object called and names
// are the class's, also when the caller is a method of another class.
class Ev;
  int id;
  int got;
  function new(int i); id = i; endfunction
  virtual function int dflt(); return 100 + id; endfunction
  function void trigger(int data = dflt(), int twice = 2 * data);
    got = data + twice;
  endfunction
endclass
class Ev2 extends Ev;
  function new(int i); super.new(i); endfunction
  virtual function int dflt(); return 200 + id; endfunction
endclass
class User;
  int id = 7;
  function int dflt(); return -1; endfunction
  function void fire(Ev e); e.trigger(); endfunction
endclass
module t;
  Ev a = new(1);
  Ev2 b = new(2);
  User u = new;
  int bad = 0;
  initial begin
    u.fire(a);
    if (a.got != 101 * 3) bad++;
    u.fire(b);
    if (b.got != 202 * 3) bad++;
    a.trigger(5);
    if (a.got != 15) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d %0d %0d", bad, a.got, b.got);
  end
endmodule
