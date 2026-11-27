// A specialization that leaves a parameter at its default names itself in its body with
// the value written out (`typedef C #(T, Tname) this_type;`): both are the same class
// (IEEE 1800-2017 8.25), so handles of one hold objects of the other.
class Base; endclass
class Reg #(type T = Base, string Tname = "<unknown>");
  typedef Reg #(T, Tname) this_type;
  static function this_type get();
    static this_type m_inst;
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
  function string name(); return Tname; endfunction
endclass
class A extends Base;
  typedef Reg #(A) type_id;
  static function type_id get_type(); return type_id::get(); endfunction
endclass
module t;
  initial begin
    Reg #(A, "<unknown>") r = A::get_type();
    if (r == A::get_type() && r.name() == "<unknown>") $display("PASSED");
    else $display("FAILED");
  end
endmodule
