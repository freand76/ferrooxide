// Enum results of methods and static properties through handles (IEEE 1800-2017 6.19.3,
// 8.6, 8.9, 8.25): a method's enum result, and one given by a type parameter, assigned to
// an enum variable without a cast; an element of an associative array property of a type
// parameter's enum type; a static property of a parameterized class read and written
// through a handle.
typedef enum bit [1:0] { A, B, C } sev_e;
typedef sev_e sev;

class M;
  protected sev s = B;
  virtual function sev get_s();
    return s;
  endfunction
  static function sev pick();
    return C;
  endfunction
endclass

class Pool #(type KEY = int, type T = int);
  protected T pool[KEY];
  function void add(KEY k, T v);
    pool[k] = v;
  endfunction
  function T get(KEY k);
    return pool[k];
  endfunction
endclass

class Cbs #(type T = int);
  static int count;
  static Cbs #(T) inst;
  static function Cbs #(T) init();
    if (inst == null) begin
      inst = new;
      inst.count = 3;
    end
    return inst;
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input bit ok);
    if (!ok) begin
      $display("FAILED line %0d", line);
      errors = errors + 1;
    end
  endtask
  initial begin
    M m = new;
    Pool #(string, sev) p = new;
    Cbs #(byte) cb;
    sev x = m.get_s();
    sev y;
    check(`__LINE__, x == B);
    x = M::pick();
    check(`__LINE__, x == C);
    p.add("k", A);
    y = p.get("k");
    check(`__LINE__, y == A);
    cb = Cbs #(byte)::init();
    check(`__LINE__, cb.count == 3 && Cbs #(byte)::count == 3);
    cb.count = 4;
    check(`__LINE__, Cbs #(byte)::count == 4);
    if (errors == 0) $display("PASSED");
  end
endmodule
