// An enum type and its constants declared in a base class (in a package) are the same
// enum in a derived class: a constant initializes and is returned as a value of the type.
package p;
  virtual class B;
    typedef enum { UNKNOWN_ACTION, THROW, CAUGHT } action_e;
    pure virtual function action_e catch_it();
  endclass
endpackage
import p::*;
class D extends B;
  function action_e catch_it();
    action_e x = CAUGHT;
    if (x != CAUGHT) return UNKNOWN_ACTION;
    return THROW;
  endfunction
endclass
module m;
  initial begin
    D d = new;
    if (d.catch_it() == B::THROW && d.catch_it().name() == "THROW") $display("PASSED");
    else $display("FAILED");
  end
endmodule
